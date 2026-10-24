// SPDX-License-Identifier: GPL-2.0
// Copyright (c) 2024 Broadcom

//! Rust port of `bcm74110-rng.c`: BCM 74110 Random Number Generator (RNG) driver.

use kernel::{
    device::Core,
    hwrng::{HwRngName, HwRngOptions, HwRngRegistration, Operations},
    io::{mem::IoMem, Io},
    of, platform,
    prelude::*,
    time::{delay, Delta},
};

#[allow(dead_code)] // Register block documented for completeness, as in the C driver.
const HOST_REV_ID: usize = 0x00;
#[allow(dead_code)] // Register block documented for completeness, as in the C driver.
const HOST_FIFO_DEPTH: usize = 0x04;
const HOST_FIFO_COUNT: usize = 0x08;
#[allow(dead_code)] // Register block documented for completeness, as in the C driver.
const HOST_FIFO_THRESHOLD: usize = 0x0c;
const HOST_FIFO_DATA: usize = 0x10;
const HOST_FIFO_COUNT_MASK: u32 = 0xffff;
const FIFO_DELAY_MIN_US: u32 = 3;
#[allow(dead_code)] // Upper bound of the delay range; only the minimum is slept.
const FIFO_DELAY_MAX_US: u32 = 7;
const FIFO_DELAY_MAX_COUNT: u32 = 10;

/// The driver data handed to the hwrng core.
struct Rng<'a> {
    mmio: IoMem<'a>,
}

fn fifo_count(mmio: &IoMem<'_>) -> Result<u32> {
    Ok(mmio.try_read32(HOST_FIFO_COUNT)? & HOST_FIFO_COUNT_MASK)
}

#[vtable]
impl<'a> Operations for Rng<'a> {
    type Ptr = KBox<Self>;

    fn read(data: &Self, buffer: &mut [u8], wait: bool) -> Result<usize> {
        let max_words = buffer.len() / 4;
        let mut num_words = fifo_count(&data.mmio)? as usize;
        let mut underrun_count = 0;

        // Check how many words are available in the RNG FIFO; if there are
        // none, wait for some to become available.
        while num_words == 0 {
            if !wait {
                return Ok(0);
            }
            // As a precaution, limit how long we wait. If the FIFO doesn't
            // refill within the allotted time, return 0 (=no data) to the
            // caller.
            if underrun_count < FIFO_DELAY_MAX_COUNT {
                delay::fsleep(Delta::from_micros(i64::from(FIFO_DELAY_MIN_US)));
            } else {
                return Ok(0);
            }
            underrun_count += 1;
            num_words = fifo_count(&data.mmio)? as usize;
        }
        num_words = num_words.min(max_words);

        // Bail early if we run out of random numbers unexpectedly.
        let mut i = 0;
        while i < num_words && fifo_count(&data.mmio)? > 0 {
            let value = data.mmio.try_read32(HOST_FIFO_DATA)?;
            buffer[i * 4..i * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            i += 1;
        }

        Ok(i * 4)
    }
}

/// The platform driver's data; fields drop in order: the hwrng is
/// unregistered before anything it uses goes away.
struct Data<'a> {
    _rng: Pin<KBox<HwRngRegistration<Rng<'a>>>>,
}

struct Driver;

kernel::of_device_table!(
    OF_TABLE,
    <Driver as platform::Driver>::IdInfo,
    [(of::DeviceId::new(c"brcm,bcm74110-rng"), ())]
);

impl platform::Driver for Driver {
    type IdInfo = ();
    type Data<'bound> = Data<'bound>;
    const OF_ID_TABLE: Option<of::IdTable<Self::IdInfo>> = Some(&OF_TABLE);

    fn probe<'bound>(
        pdev: &'bound platform::Device<Core<'_>>,
        id_info: Option<&'bound Self::IdInfo>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        let _ = id_info;
        let dev = pdev.as_ref();
        let mmio = pdev.io_request_by_index(0).ok_or(ENODEV)?.iomap()?;
        let rng = KBox::new(Rng { mmio }, GFP_KERNEL)?;
        let name = HwRngName::Owned(dev.name().to_cstring()?);
        let opts = HwRngOptions { name, quality: 0 };
        let rng = KBox::pin_init(HwRngRegistration::register(opts, rng), GFP_KERNEL)?;
        Ok(Data { _rng: rng })
    }
}

kernel::module_platform_driver! {
    type: Driver,
    name: "bcm74110-rng",
    authors: ["Markus Mayer <mmayer@broadcom.com>"],
    description: "BCM 74110 Random Number Generator (RNG) driver",
    license: "GPL v2",
}
