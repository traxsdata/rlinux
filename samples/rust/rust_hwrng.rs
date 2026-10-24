// SPDX-License-Identifier: GPL-2.0

//! Rust hwrng sample.
//!
//! Registers a hardware RNG named `rust_hwrng` that returns a deterministic xorshift64 stream.
//! It is **not** a source of entropy (its quality is 0); it exists to exercise the
//! registration, callback and unregistration paths of [`kernel::hwrng`].
//!
//! To try it: `echo rust_hwrng > /sys/class/misc/hw_random/rng_current`, then read
//! `/dev/hwrng`.

use kernel::{
    hwrng::{HwRngName, HwRngOptions, HwRngRegistration, Operations},
    prelude::*,
    sync::atomic::{Atomic, Relaxed},
};

module! {
    type: RustHwRng,
    name: "rust_hwrng",
    authors: ["Rust for Linux Contributors"],
    description: "Rust hwrng sample",
    license: "GPL",
}

struct SampleRng {
    state: Atomic<u64>,
}

impl SampleRng {
    /// Advance the xorshift64 state and return the next value.
    ///
    /// Concurrent readers may see the same value; that is fine for a sample with no entropy.
    fn next(&self) -> u64 {
        let mut x = self.state.load(Relaxed);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state.store(x, Relaxed);
        x
    }
}

#[vtable]
impl Operations for SampleRng {
    type Ptr = KBox<Self>;

    fn init(_rng: &SampleRng) -> Result {
        pr_info!("rust_hwrng: init\n");
        Ok(())
    }

    fn cleanup(_rng: &SampleRng) {
        pr_info!("rust_hwrng: cleanup\n");
    }

    fn read(rng: &SampleRng, buffer: &mut [u8], _wait: bool) -> Result<usize> {
        for chunk in buffer.chunks_mut(8) {
            let bytes = rng.next().to_ne_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(buffer.len())
    }
}

struct RustHwRng {
    _rng: Pin<KBox<HwRngRegistration<SampleRng>>>,
}

impl kernel::Module for RustHwRng {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        let data = KBox::new(
            SampleRng {
                state: Atomic::new(0x2545_f491_4f6c_dd1d),
            },
            GFP_KERNEL,
        )?;
        let opts = HwRngOptions {
            name: HwRngName::Static(c"rust_hwrng"),
            quality: 0,
        };
        let rng = KBox::pin_init(
            HwRngRegistration::<SampleRng>::register(opts, data),
            GFP_KERNEL,
        )?;
        pr_info!("rust_hwrng: registered\n");
        Ok(Self { _rng: rng })
    }
}
