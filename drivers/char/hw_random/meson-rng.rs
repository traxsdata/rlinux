// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
/*
 * Copyright (c) 2016 BayLibre, SAS.
 * Author: Neil Armstrong <narmstrong@baylibre.com>
 * Copyright (C) 2014 Amlogic, Inc.
 */

//! Rust port of `meson-rng.c`: the Amlogic Meson SoC hardware random number
//! generator driver.
//!
//! The hwrng is registered while the platform driver is bound to a device
//! and unregistered when it is unbound: the platform driver's data holds
//! the [`HwRngRegistration`] (which owns the driver data passed to the
//! hwrng core, including the I/O mapping), and, when the common clock
//! framework is available, it additionally holds the enabled "core" clock.

use kernel::{
    device::Core,
    hwrng::{HwRngName, HwRngOptions, HwRngRegistration, Operations},
    io::{
        mem::IoMem,
        poll,
        Io, //
    },
    of, platform,
    prelude::*,
    sync::aref::ARef,
    time::Delta,
};

#[cfg(CONFIG_COMMON_CLK)]
use kernel::clk::OptionalClk;

// Register offsets
const RNG_DATA: usize = 0x00;
const RNG_S4_DATA: usize = 0x08;
const RNG_S4_CFG: usize = 0x00;

// `RNG_S4_CFG` bits
const RUN_BIT: u32 = 0x0000_0001;
const SEED_READY_STS_BIT: u32 = 0x8000_0000;

/// The driver data handed to the hwrng core: a device reference used for
/// diagnostics, the I/O mapping, and whether this IP version is the Meson S4
/// one, whose random words must be read after a seeding handshake.
struct MesonRng<'a> {
    dev: ARef<platform::Device>,
    mmio: IoMem<'a>,
    s4: bool,
}

impl MesonRng<'_> {
    /// Wait for `bit` in `RNG_S4_CFG` to be cleared, up to 10 ms.
    ///
    /// Port of `meson_rng_wait_status`: the C driver polls with
    /// `readl_relaxed_poll_timeout_atomic(cfg, status, !(status & bit), 10,
    /// 10000)` — every 10 us for up to 10 ms — and maps any failure to
    /// `-EBUSY`.
    fn wait_status(io: &IoMem<'_>, bit: u32) -> Result {
        poll::read_poll_timeout_atomic(
            || io.try_read32(RNG_S4_CFG),
            |val: &u32| val & bit == 0,
            Delta::from_micros(10),
            1000,
        )
        .map(|_| ())
        .map_err(|_| EBUSY)
    }

    /// Port of `meson_s4_rng_read`: request a new seed, wait for the
    /// seed-ready and run bits to clear, then read the `RNG_S4_DATA` word.
    fn s4_read(&self) -> Result<u32> {
        let cfg = self.mmio.try_read32(RNG_S4_CFG)?;
        self.mmio
            .try_write32(cfg | SEED_READY_STS_BIT, RNG_S4_CFG)?;

        if let Err(err) = Self::wait_status(&self.mmio, SEED_READY_STS_BIT) {
            dev_err!(self.dev, "Seed isn't ready, try again\n");
            return Err(err);
        }

        if let Err(err) = Self::wait_status(&self.mmio, RUN_BIT) {
            dev_err!(self.dev, "Can't get random number, try again\n");
            return Err(err);
        }

        self.mmio.try_read32(RNG_S4_DATA)
    }

    /// Port of `meson_rng_read`: read the `RNG_DATA` word directly.
    fn read32(&self) -> Result<u32> {
        self.mmio.try_read32(RNG_DATA)
    }
}

#[vtable]
impl<'a> Operations for MesonRng<'a> {
    type Ptr = KBox<Self>;

    fn read(data: &Self, buffer: &mut [u8], _wait: bool) -> Result<usize> {
        // The C driver always stores one `u32`; a shorter buffer can't hold
        // one, so report no data instead of overflowing it.
        if buffer.len() < 4 {
            return Ok(0);
        }

        let value = if data.s4 {
            data.s4_read()?
        } else {
            data.read32()?
        };

        buffer[..4].copy_from_slice(&value.to_ne_bytes());
        Ok(4)
    }
}

/// Holds the enabled "core" clock until the device is unbound.
#[cfg(CONFIG_COMMON_CLK)]
struct EnabledClk(OptionalClk);

#[cfg(CONFIG_COMMON_CLK)]
impl Drop for EnabledClk {
    fn drop(&mut self) {
        // Release path of the C driver's devm_clk_get_optional_enabled:
        // disable and unprepare the clock before the reference count is
        // released (the `OptionalClk` drop releases it).
        self.0.disable_unprepare();
    }
}

/// The platform driver's data for a bound device.
///
/// Dropping it (when the device is unbound) first unregisters the hwrng —
/// which waits for in-flight `read` callbacks and releases the driver data
/// holding the I/O mapping — and then releases the "core" clock, mirroring
/// the C driver's devres unwind order.
struct MesonRngData<'a> {
    _rng: Pin<KBox<HwRngRegistration<MesonRng<'a>>>>,
    #[cfg(CONFIG_COMMON_CLK)]
    _clk: EnabledClk,
}

struct MesonRngDriver;

kernel::of_device_table!(
    OF_TABLE,
    <MesonRngDriver as platform::Driver>::IdInfo,
    [
        // `false` = plain Meson IP, `true` = the Meson S4 variant that needs
        // the seeding handshake
        (of::DeviceId::new(c"amlogic,meson-rng"), false),
        (of::DeviceId::new(c"amlogic,meson-s4-rng"), true)
    ]
);

impl platform::Driver for MesonRngDriver {
    type IdInfo = bool;
    type Data<'bound> = MesonRngData<'bound>;
    const OF_ID_TABLE: Option<of::IdTable<Self::IdInfo>> = Some(&OF_TABLE);

    fn probe<'bound>(
        pdev: &'bound platform::Device<Core<'_>>,
        info: Option<&'bound Self::IdInfo>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        let s4 = *info.ok_or(ENODEV)?;

        let dev = pdev.as_ref();
        let request = pdev.io_request_by_index(0).ok_or(ENODEV)?;
        let mmio = request.iomap()?;

        #[cfg(CONFIG_COMMON_CLK)]
        let clk = {
            let clk = match OptionalClk::get(pdev.as_ref(), Some(c"core")) {
                Ok(clk) => clk,
                Err(err) => {
                    dev_err!(dev, "Failed to get core clock\n");
                    return Err(err);
                }
            };

            // `OptionalClk` (like the C `devm_clk_get_optional_enabled`) works
            // with an absent "core" clock: `clk_prepare_enable` is a no-op on
            // the null clock and returns success.
            if let Err(err) = clk.prepare_enable() {
                dev_err!(dev, "Failed to get core clock\n");
                return Err(err);
            }

            EnabledClk(clk)
        };

        let trng = KBox::new(
            MesonRng {
                dev: pdev.into(),
                mmio,
                s4,
            },
            GFP_KERNEL,
        )?;

        // Register the hwrng under the device name, like the C driver's
        // `rng.name = pdev->name`; the registration owns the name buffer for
        // the bound scope. The C driver never sets a quality, so keep it 0.
        let name = HwRngName::Owned(dev.name().to_cstring()?);

        let rng = KBox::pin_init(
            HwRngRegistration::register(HwRngOptions { name, quality: 0 }, trng),
            GFP_KERNEL,
        )?;

        Ok(MesonRngData {
            _rng: rng,
            #[cfg(CONFIG_COMMON_CLK)]
            _clk: clk,
        })
    }
}

kernel::module_platform_driver! {
    type: MesonRngDriver,
    name: "meson-rng",
    authors: ["Lawrence Mok <lawrence.mok@amlogic.com>", "Neil Armstrong <narmstrong@baylibre.com>"],
    description: "Meson H/W Random Number Generator driver",
    license: "Dual BSD/GPL",
}
