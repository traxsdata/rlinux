// SPDX-License-Identifier: GPL-2.0
// Copyright (C) 2015 STMicroelectronics (R&D) Limited
//
// Author: Pankaj Dev: <pankaj.dev@st.com>
//         Lee Jones <lee.jones@linaro.org>

//! Rust ST Random Number Generator driver.
//!
//! Port of `st-rng.c` for ST's platforms. The hwrng is registered while the
//! platform driver is bound to a device and unregistered when the device is
//! unbound: the platform driver's data holds both the [`HwRngRegistration`]
//! and the device clock, and drops them so that the clock is disabled and
//! unprepared only after the hwrng has been unregistered (and all in-flight
//! `read` callbacks have completed) — the same order as the C driver's
//! devres unwind.

use kernel::{
    clk::Clk,
    device::Core,
    hwrng::{HwRngName, HwRngOptions, HwRngRegistration, Operations},
    io::{
        mem::IoMem,
        Io, //
    },
    of, platform,
    prelude::*,
    time::{
        delay,
        Delta, //
    },
};

/* Registers */
const ST_RNG_STATUS_REG: usize = 0x20;
const ST_RNG_DATA_REG: usize = 0x24;

/* Registers fields */
// The two error-status bits are documented for register completeness, as in
// the C driver, but are not tested by the read path.
#[allow(dead_code)]
const ST_RNG_STATUS_BAD_SEQUENCE: u32 = 1 << 0;
#[allow(dead_code)]
const ST_RNG_STATUS_BAD_ALTERNANCE: u32 = 1 << 1;
const ST_RNG_STATUS_FIFO_FULL: u32 = 1 << 5;

const ST_RNG_SAMPLE_SIZE: usize = 2; // 2 Byte (16bit) samples
const ST_RNG_FIFO_DEPTH: usize = 4;
const ST_RNG_FIFO_SIZE: usize = ST_RNG_FIFO_DEPTH * ST_RNG_SAMPLE_SIZE;

/*
 * Samples are documented to be available every 0.667us, so in theory
 * the 4 sample deep FIFO should take 2.668us to fill.  However, during
 * thorough testing, it became apparent that filling the FIFO actually
 * takes closer to 12us.  We then multiply by 2 in order to account for
 * the lack of udelay()'s reliability, suggested by Russell King.
 */
const ST_RNG_FILL_FIFO_TIMEOUT: usize = 12 * 2;

/// The driver data handed to the hwrng core; the FIFO is waited on until full,
/// then its 2-byte samples are read out into the caller's buffer.
struct StRng<'a> {
    mmio: IoMem<'a>,
}

#[vtable]
impl<'a> Operations for StRng<'a> {
    type Ptr = KBox<Self>;

    fn read(data: &Self, buffer: &mut [u8], _wait: bool) -> Result<usize> {
        let mut full = false;

        /* Wait until FIFO is full - max 4uS*/
        for _ in 0..ST_RNG_FILL_FIFO_TIMEOUT {
            let status = data.mmio.try_read32(ST_RNG_STATUS_REG)?;
            if status & ST_RNG_STATUS_FIFO_FULL != 0 {
                full = true;
                break;
            }
            delay::udelay(Delta::from_micros(1));
        }

        if !full {
            return Ok(0);
        }

        // Read up to FIFO_SIZE two-byte samples from the FIFO: each sample is a
        // 32-bit read whose low 16 bits form the sample, in native byte order.
        let mut i = 0;
        while i < ST_RNG_FIFO_SIZE && i + ST_RNG_SAMPLE_SIZE <= buffer.len() {
            let value = data.mmio.try_read32(ST_RNG_DATA_REG)?;
            buffer[i..i + ST_RNG_SAMPLE_SIZE].copy_from_slice(&(value as u16).to_ne_bytes());
            i += ST_RNG_SAMPLE_SIZE;
        }

        Ok(i) // No of bytes read
    }
}

/// A prepared and enabled clock that is disabled and unprepared on drop.
///
/// `Clk`'s `Drop` puts the reference but does not release the
/// prepare/enable counts, so the wrapper performs that. It is what makes the
/// fields of [`StRngData`] drop in the right order, without any `unsafe`:
/// Rust drops the fields in declaration order, so the hwrng registration is
/// unregistered (and its driver data, holding the I/O mapping, freed) before
/// the clock is disabled and unprepared — the clock stays on for as long as a
/// `read` callback could touch the device.
struct ClkOn(Clk);

impl Drop for ClkOn {
    fn drop(&mut self) {
        self.0.disable_unprepare();
        // The `Clk` itself is then dropped, putting the reference.
    }
}

/// The platform driver's data for a bound device.
///
/// Dropping it (when the device is unbound) unregisters the hwrng and only
/// then disables and unprepares the clock, so in-flight `read` callbacks can
/// never see the clock turn off.
struct StRngData<'a> {
    _rng: Pin<KBox<HwRngRegistration<StRng<'a>>>>,
    // `_clk` is never read; it is kept alive so that the clock is disabled
    // and unprepared only once the hwrng registration above it has been
    // dropped (fields drop in declaration order).
    _clk: ClkOn,
}

struct StRngDriver;

kernel::of_device_table!(
    OF_TABLE,
    <StRngDriver as platform::Driver>::IdInfo,
    [(of::DeviceId::new(c"st,rng"), ())]
);

impl platform::Driver for StRngDriver {
    type IdInfo = ();
    type Data<'bound> = StRngData<'bound>;
    const OF_ID_TABLE: Option<of::IdTable<Self::IdInfo>> = Some(&OF_TABLE);

    fn probe<'bound>(
        pdev: &'bound platform::Device<Core<'_>>,
        _id_info: Option<&'bound Self::IdInfo>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        let dev = pdev.as_ref();

        let request = pdev.io_request_by_index(0).ok_or(ENODEV)?;
        let mmio = request.iomap()?;

        let clk = ClkOn(Clk::get(dev, None)?);
        clk.0.prepare_enable()?;

        let rng = KBox::new(StRng { mmio }, GFP_KERNEL)?;

        // Register the hwrng under the device name (C: `ops.name = pdev->name`):
        // that is what userspace sees and switches with `rng_current`, and it
        // keeps the name unique per device, while the registration owns the
        // buffer for the bound scope.
        let name = HwRngName::Owned(dev.name().to_cstring()?);

        let rng = match KBox::pin_init(
            HwRngRegistration::register(HwRngOptions { name, quality: 0 }, rng),
            GFP_KERNEL,
        ) {
            Ok(rng) => rng,
            Err(err) => {
                dev_err!(dev, "Failed to register HW RNG\n");
                return Err(err);
            }
        };

        dev_info!(dev, "Successfully registered HW RNG\n");

        Ok(StRngData {
            _rng: rng,
            _clk: clk,
        })
    }
}

kernel::module_platform_driver! {
    type: StRngDriver,
    name: "st-hwrandom",
    authors: ["Pankaj Dev", "Lee Jones <lee.jones@linaro.org>"],
    description: "ST Microelectronics HW Random Number Generator",
    license: "GPL v2",
}
