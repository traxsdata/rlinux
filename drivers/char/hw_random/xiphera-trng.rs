// SPDX-License-Identifier: GPL-2.0
// Copyright (C) 2020 Xiphera Ltd.

//! Rust Xiphera FPGA-based true random number generator driver.
//!
//! Port of `xiphera-trng.c`. The hwrng is registered while the platform
//! driver is bound to a device and unregistered when the device is unbound:
//! the platform driver's data holds both the [`HwRngRegistration`] and (via
//! the driver data passed to the hwrng core) the I/O mapping, and both are
//! released together.

use kernel::{
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

// Register offsets
const CONTROL_REG: usize = 0x0000_0000;
const STATUS_REG: usize = 0x0000_0004;
const RAND_REG: usize = 0x0000_0000;

// Control register values
const HOST_TO_TRNG_RESET: u32 = 0x0000_0001;
const HOST_TO_TRNG_RELEASE_RESET: u32 = 0x0000_0002;
const HOST_TO_TRNG_ENABLE: u32 = 0x8000_0000;
const HOST_TO_TRNG_ZEROIZE: u32 = 0x8000_0004;
const HOST_TO_TRNG_ACK_ZEROIZE: u32 = 0x8000_0008;
const HOST_TO_TRNG_READ: u32 = 0x8000_000F;

// TRNG statuses
const TRNG_ACK_RESET: u32 = 0x0000_00AC;
const TRNG_SUCCESSFUL_STARTUP: u32 = 0x0000_0057;
const TRNG_FAILED_STARTUP: u32 = 0x0000_00FA;
const TRNG_NEW_RAND_AVAILABLE: u32 = 0x0000_00ED;

/// The driver data handed to the hwrng core; its `read` callback fills the
/// caller's buffer with the hardware's random words as long as the TRNG
/// reports a new random number as available.
struct XipheraTrng<'a> {
    mmio: IoMem<'a>,
}

#[vtable]
impl<'a> Operations for XipheraTrng<'a> {
    type Ptr = KBox<Self>;

    fn read(data: &Self, buffer: &mut [u8], _wait: bool) -> Result<usize> {
        let mut ret = 0;
        let mut offset = 0;

        while buffer.len() - offset >= 4 {
            if data.mmio.try_read32(STATUS_REG)? != TRNG_NEW_RAND_AVAILABLE {
                break;
            }

            let value = data.mmio.try_read32(RAND_REG)?;
            // Inform the trng of the read and re-enable it to produce a new
            // random number.
            data.mmio.try_write32(HOST_TO_TRNG_READ, CONTROL_REG)?;
            data.mmio.try_write32(HOST_TO_TRNG_ENABLE, CONTROL_REG)?;

            buffer[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
            offset += 4;
            ret += 4;
        }

        Ok(ret)
    }
}

/// The platform driver's data for a bound device.
///
/// Dropping it (when the device is unbound) unregisters the hwrng, and the
/// hwrng core guarantees all callbacks have finished before the driver data
/// (holding the I/O mapping) is freed.
///
/// The `rng` field value is never read; it is kept alive so that the hwrng
/// stays registered (and unregistered on drop) for the whole bound scope.
struct XipheraTrngData<'a> {
    _rng: Pin<KBox<HwRngRegistration<XipheraTrng<'a>>>>,
}

struct XipheraTrngDriver;

kernel::of_device_table!(
    OF_TABLE,
    <XipheraTrngDriver as platform::Driver>::IdInfo,
    [(of::DeviceId::new(c"xiphera,xip8001b-trng"), ())]
);

impl platform::Driver for XipheraTrngDriver {
    type IdInfo = ();
    type Data<'bound> = XipheraTrngData<'bound>;
    const OF_ID_TABLE: Option<of::IdTable<Self::IdInfo>> = Some(&OF_TABLE);

    fn probe<'bound>(
        pdev: &'bound platform::Device<Core<'_>>,
        _id_info: Option<&'bound Self::IdInfo>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        let dev = pdev.as_ref();

        let request = pdev.io_request_by_index(0).ok_or(ENODEV)?;
        let mmio = request.iomap()?;

        /*
         * the trng needs to be reset first which might not happen in time,
         * hence we incorporate a small delay to ensure proper behaviour
         */
        mmio.try_write32(HOST_TO_TRNG_RESET, CONTROL_REG)?;
        delay::fsleep(Delta::from_micros(100));

        if mmio.try_read32(STATUS_REG)? != TRNG_ACK_RESET {
            /*
             * there is a small chance the trng is just not ready yet,
             * so we try one more time. If the second time fails, we give up
             */
            delay::fsleep(Delta::from_micros(100));
            if mmio.try_read32(STATUS_REG)? != TRNG_ACK_RESET {
                dev_err!(dev, "failed to reset the trng ip\n");
                return Err(ENODEV);
            }
        }

        /*
         * once again, to ensure proper behaviour we sleep
         * for a while after zeroizing the trng
         */
        mmio.try_write32(HOST_TO_TRNG_RELEASE_RESET, CONTROL_REG)?;
        mmio.try_write32(HOST_TO_TRNG_ENABLE, CONTROL_REG)?;
        mmio.try_write32(HOST_TO_TRNG_ZEROIZE, CONTROL_REG)?;
        delay::fsleep(Delta::from_millis(20));

        if mmio.try_read32(STATUS_REG)? != TRNG_SUCCESSFUL_STARTUP {
            // Diagnose the reason for the failure
            if mmio.try_read32(STATUS_REG)? == TRNG_FAILED_STARTUP {
                dev_err!(dev, "trng ip startup-tests failed\n");
            } else {
                dev_err!(dev, "startup-tests yielded no response\n");
            }
            return Err(ENODEV);
        }

        mmio.try_write32(HOST_TO_TRNG_ACK_ZEROIZE, CONTROL_REG)?;

        let trng = KBox::new(XipheraTrng { mmio }, GFP_KERNEL)?;

        // Register the hwrng under the device name (e.g. "43c00000.trng"): that is what
        // userspace sees and switches with rng_current, and it keeps the name unique per
        // device, while the registration owns the buffer for the bound scope.
        let name = HwRngName::Owned(dev.name().to_cstring()?);

        let rng = match KBox::pin_init(
            HwRngRegistration::register(HwRngOptions { name, quality: 900 }, trng),
            GFP_KERNEL,
        ) {
            Ok(rng) => rng,
            Err(err) => {
                dev_err!(dev, "failed to register rng device: {err:?}\n");
                return Err(err);
            }
        };

        Ok(XipheraTrngData { _rng: rng })
    }
}

kernel::module_platform_driver! {
    type: XipheraTrngDriver,
    name: "xiphera-trng",
    authors: ["Atte Tommiska"],
    description: "Xiphera FPGA-based true random number generator driver",
    license: "GPL",
}
