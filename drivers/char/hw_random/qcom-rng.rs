// SPDX-License-Identifier: GPL-2.0
// Copyright (c) 2017-18 Linaro Limited

//! Rust port of `qcom-rng.c`: the Qualcomm true random number generator
//! driver.
//!
//! The hwrng is registered while the platform driver is bound to a device
//! and unregistered when it is unbound: the platform driver's data holds the
//! [`HwRngRegistration`] (which owns the driver data handed to the hwrng
//! core, including the I/O mapping), and, when the common clock framework is
//! available, it additionally holds the prepared and enabled "core" clock —
//! the C driver enables it in its hwrng `init` callback at registration
//! (probe here) and disables it in the `cleanup` callback at unregistration
//! (drop here). Only devices whose match data sets `hwrng_support` (i.e.
//! `qcom,trng`) get an hwrng registered; the others are bound with nothing
//! else, like the C driver.

#[cfg(CONFIG_COMMON_CLK)]
use kernel::clk::OptionalClk;
use kernel::{
    acpi,
    device::Core,
    hwrng::{HwRngName, HwRngOptions, HwRngRegistration, Operations},
    io::{
        mem::IoMem,
        poll,
        Io, //
    },
    of, platform,
    prelude::*,
    time::Delta,
};

// Device specific register offsets; the two configuration registers are
// defined for register completeness, like in the C driver, but are not
// accessed by the driver.
const PRNG_DATA_OUT: usize = 0x0000;
const PRNG_STATUS: usize = 0x0004;
#[allow(dead_code)]
const PRNG_LFSR_CFG: usize = 0x0100;
#[allow(dead_code)]
const PRNG_CONFIG: usize = 0x0104;

// Device specific register masks and config values; documented for register
// completeness, like in the C driver, but not used by the read path.
#[allow(dead_code)]
const PRNG_LFSR_CFG_MASK: u32 = 0x0000ffff;
#[allow(dead_code)]
const PRNG_LFSR_CFG_CLOCKS: u32 = 0x0000dddd;
#[allow(dead_code)]
const PRNG_CONFIG_HW_ENABLE: u32 = 1 << 1;
const PRNG_STATUS_DATA_AVAIL: u32 = 1 << 0;

const WORD_SZ: usize = 4;

const QCOM_TRNG_QUALITY: u16 = 1024;

/// The driver data handed to the hwrng core: the I/O mapping used by the read
/// callback to fetch random words from the hardware.
struct QcomRng<'a> {
    mmio: IoMem<'a>,
}

#[vtable]
impl<'a> Operations for QcomRng<'a> {
    type Ptr = KBox<Self>;

    /// Port of `qcom_hwrng_read`/`qcom_rng_read`: wait until the
    /// `PRNG_STATUS_DATA_AVAIL` bit is set, read a word from
    /// `PRNG_DATA_OUT`, and copy it (or its remaining bytes if the buffer is
    /// almost full) until the buffer is full.
    fn read(data: &Self, buffer: &mut [u8], _wait: bool) -> Result<usize> {
        let max = buffer.len();
        let mut currsize = 0usize;

        while currsize < max {
            // The C driver polls with `readl_poll_timeout(base + PRNG_STATUS,
            // val, val & PRNG_STATUS_DATA_AVAIL, 200, 10000)` — every 200 us
            // for at most 10000 reads.
            poll::read_poll_timeout(
                || data.mmio.try_read32(PRNG_STATUS),
                |val: &u32| *val & PRNG_STATUS_DATA_AVAIL != 0,
                Delta::from_micros(200),
                Delta::from_millis(2000),
            )?;

            let val = data.mmio.try_read32(PRNG_DATA_OUT)?;

            if max - currsize >= WORD_SZ {
                buffer[currsize..currsize + WORD_SZ].copy_from_slice(&val.to_ne_bytes());
                currsize += WORD_SZ;
            } else {
                // Copy only the remaining bytes, like the C driver.
                buffer[currsize..max].copy_from_slice(&val.to_ne_bytes()[..max - currsize]);
                currsize = max;
            }
        }

        Ok(currsize)
    }
}

/// Holds the prepared and enabled "core" clock until the device is unbound.
#[cfg(CONFIG_COMMON_CLK)]
struct EnabledClk(OptionalClk);

#[cfg(CONFIG_COMMON_CLK)]
impl Drop for EnabledClk {
    fn drop(&mut self) {
        // Release path of the C driver's `clk_prepare_enable` at registration
        // time: disable and unprepare the clock before the reference count is
        // released (which happens when the `OptionalClk` is dropped).
        self.0.disable_unprepare();
    }
}

/// The platform driver's data for a bound device.
///
/// Dropping it (when the device is unbound) first unregisters the hwrng —
/// which waits for in-flight `read` callbacks and releases the driver data
/// holding the I/O mapping — and only then disables and unprepares the
/// "core" clock, mirroring the C driver's devres unwind order. When the
/// device's match data has `hwrng_support` unset, the fields are simply
/// dropped without having been set up.
struct QcomRngData<'a> {
    _rng: Option<Pin<KBox<HwRngRegistration<QcomRng<'a>>>>>,
    #[cfg(CONFIG_COMMON_CLK)]
    _clk: Option<EnabledClk>,
}

struct QcomRngDriver;

kernel::of_device_table!(
    OF_TABLE,
    <QcomRngDriver as platform::Driver>::IdInfo,
    [
        // The C `qcom_rng_of_match` table, with the `.data` pointer's
        // `hwrng_support` field.
        (of::DeviceId::new(c"qcom,prng"), false),
        (of::DeviceId::new(c"qcom,prng-ee"), false),
        (of::DeviceId::new(c"qcom,trng"), true)
    ]
);

kernel::acpi_device_table!(
    ACPI_TABLE,
    <QcomRngDriver as platform::Driver>::IdInfo,
    [
        // The C `qcom_rng_acpi_match` table: `QCOM8160` uses the `qcom,prng-ee`
        // match data, whose `hwrng_support` is unset.
        (acpi::DeviceId::new(c"QCOM8160"), false)
    ]
);

impl platform::Driver for QcomRngDriver {
    type IdInfo = bool;
    type Data<'bound> = QcomRngData<'bound>;
    const OF_ID_TABLE: Option<of::IdTable<Self::IdInfo>> = Some(&OF_TABLE);
    const ACPI_ID_TABLE: Option<acpi::IdTable<Self::IdInfo>> = Some(&ACPI_TABLE);

    fn probe<'bound>(
        pdev: &'bound platform::Device<Core<'_>>,
        info: Option<&'bound bool>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        let dev = pdev.as_ref();

        // The C driver's `device_get_match_data()` being NULL, or
        // `hwrng_support` being unset: only report it and keep the device
        // bound, so the driver's presence has no other effect.
        let supported = matches!(info, Some(v) if *v);
        if !supported {
            dev_info!(dev, "TRNG support not detected\n");
            return Ok(QcomRngData {
                _rng: None,
                #[cfg(CONFIG_COMMON_CLK)]
                _clk: None,
            });
        }

        let request = pdev.io_request_by_index(0).ok_or(ENODEV)?;
        let mmio = request.iomap()?;

        #[cfg(CONFIG_COMMON_CLK)]
        let clk = {
            // The C driver's `devm_clk_get_optional(&pdev->dev, "core")`:
            // `OptionalClk` works with an absent "core" clock, and for a
            // clock that is there `prepare_enable` can still fail.
            let clk = OptionalClk::get(dev, Some(c"core"))?;
            clk.prepare_enable()?;
            // Wrap only once enabled: if `prepare_enable()` had failed,
            // dropping an `EnabledClk` would disable/unprepare a clock that
            // was never enabled.
            EnabledClk(clk)
        };

        let rng = KBox::new(QcomRng { mmio }, GFP_KERNEL)?;

        // The C driver registers under the fixed name `qcom_hwrng` (not the
        // device name) and reports the TRNG's quality.
        let name = HwRngName::Static(c"qcom_hwrng");

        let rng = match KBox::pin_init(
            HwRngRegistration::register(
                HwRngOptions {
                    name,
                    quality: QCOM_TRNG_QUALITY,
                },
                rng,
            ),
            GFP_KERNEL,
        ) {
            Ok(rng) => rng,
            Err(err) => {
                dev_err!(dev, "Register hwrng failed: {:?}\n", err);
                return Err(err);
            }
        };

        Ok(QcomRngData {
            _rng: Some(rng),
            #[cfg(CONFIG_COMMON_CLK)]
            _clk: Some(clk),
        })
    }
}

kernel::module_platform_driver! {
    type: QcomRngDriver,
    name: "qcom-rng",
    authors: ["Linaro Limited"],
    description: "Qualcomm random number generator driver",
    license: "GPL v2",
}
