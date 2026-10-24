// SPDX-License-Identifier: GPL-2.0

//! Hardware random number generator (hwrng) support.
//!
//! C header: [`include/linux/hw_random.h`](srctree/include/linux/hw_random.h)
//!
//! Reference: <https://docs.kernel.org/admin-guide/hw_random.html>

use crate::{
    bindings,
    error::to_result,
    prelude::*,
    str::as_char_ptr_in_const_context,
    types::{ForeignOwnable, Opaque},
};
use core::marker::PhantomData;

/// Options for registering a hwrng.
#[derive(Copy, Clone)]
pub struct HwRngOptions {
    /// The unique name of the RNG, as shown in `/sys/class/misc/hw_random/rng_available`.
    pub name: &'static CStr,

    /// Estimate of the true entropy in the RNG's bitstream, in bits of entropy per 1024 bits of
    /// output.
    ///
    /// Valid values are `1` to `1024`; `0` means "unknown" and the RNG is then not used to feed
    /// the kernel's entropy pool unless the `default_quality` module parameter says otherwise.
    pub quality: u16,
}

impl HwRngOptions {
    /// Create a raw `struct hwrng` for these options, with the callbacks of `T`.
    ///
    /// The `priv` field is left zero; [`HwRngRegistration::register`] fills it in.
    pub const fn into_raw<T: Operations>(self) -> bindings::hwrng {
        let mut result: bindings::hwrng = pin_init::zeroed();
        result.name = as_char_ptr_in_const_context(self.name);
        result.quality = self.quality;
        result.read = Some(HwRngVTable::<T>::read);
        result.init = if T::HAS_INIT {
            Some(HwRngVTable::<T>::init)
        } else {
            None
        };
        result.cleanup = if T::HAS_CLEANUP {
            Some(HwRngVTable::<T>::cleanup)
        } else {
            None
        };
        result
    }
}

/// A registered hwrng.
///
/// The RNG is unregistered when this is dropped. Unlike `devm_hwrng_register()`, the lifetime of
/// the registration is not tied to a device: the owner of this value decides when the RNG goes
/// away, and dropping it always waits for every callback to finish before the driver data is
/// freed.
///
/// # Invariants
///
/// - `inner` contains a `struct hwrng` that was successfully registered with `hwrng_register()`
///   and stays registered until [`PinnedDrop::drop`] calls `hwrng_unregister()`.
/// - Its `priv` field holds a pointer returned by `<T::Ptr as ForeignOwnable>::into_foreign()`,
///   which is converted back with `from_foreign()` exactly once, in [`PinnedDrop::drop`].
#[pin_data(PinnedDrop)]
pub struct HwRngRegistration<T: Operations> {
    #[pin]
    inner: Opaque<bindings::hwrng>,
    _t: PhantomData<T>,
}

// SAFETY: `hwrng_unregister()` may be called from any thread, and the driver data is
// `T::Ptr: Send`.
unsafe impl<T: Operations> Send for HwRngRegistration<T> {}

// SAFETY: The only `&self` method returns a raw pointer; the callbacks only get shared access to
// the driver data, which is `T::Ptr: Sync`.
unsafe impl<T: Operations> Sync for HwRngRegistration<T> {}

impl<T: Operations> HwRngRegistration<T> {
    /// Register a hwrng, transferring ownership of `data` to it.
    ///
    /// The callbacks of `T` get `data` borrowed. It is dropped when the returned registration is
    /// dropped, or right away if the registration fails or the initializer is never run.
    ///
    /// # Errors
    ///
    /// Returns the error of `hwrng_register()`, e.g. `EEXIST` if a hwrng with the same name is
    /// already registered.
    pub fn register(opts: HwRngOptions, data: T::Ptr) -> impl PinInit<Self, Error> {
        try_pin_init!(Self {
            inner <- Opaque::try_ffi_init(move |slot: *mut bindings::hwrng| {
                let mut rng = opts.into_raw::<T>();
                let foreign = data.into_foreign();
                rng.priv_ = foreign as c_ulong;

                // SAFETY: `slot` is valid for writes of a `struct hwrng`, per the contract of
                // `try_ffi_init`.
                unsafe { slot.write(rng) };

                // SAFETY: `slot` is a fully initialised, pinned `struct hwrng` whose `name`
                // points to a `'static` string and whose `read` callback is set.
                let ret = to_result(unsafe { bindings::hwrng_register(slot) });
                if ret.is_err() {
                    // SAFETY: `foreign` came from `into_foreign()` above, and the RNG was not
                    // registered, so no callback can use it; this is the only conversion back.
                    drop(unsafe { T::Ptr::from_foreign(foreign) });
                }
                ret
            }),
            _t: PhantomData,
        })
    }

    /// Returns a raw pointer to the underlying `struct hwrng`.
    pub fn as_raw(&self) -> *mut bindings::hwrng {
        self.inner.get()
    }
}

#[pinned_drop]
impl<T: Operations> PinnedDrop for HwRngRegistration<T> {
    fn drop(self: Pin<&mut Self>) {
        let raw = self.inner.get();

        // SAFETY: By the type invariants, `raw` is a registered `struct hwrng`.
        // `hwrng_unregister()` returns only after the RNG's `cleanup` callback (if any) has
        // completed, and no callback runs after that.
        unsafe { bindings::hwrng_unregister(raw) };

        // SAFETY: `raw` is still valid memory owned by `self`.
        let foreign = unsafe { (*raw).priv_ } as *mut c_void;

        // SAFETY: By the type invariants, `foreign` came from `into_foreign()` and this is the
        // only conversion back; the RNG is unregistered, so no callback can still borrow it.
        drop(unsafe { T::Ptr::from_foreign(foreign) });
    }
}

/// The callbacks of a hwrng.
#[vtable]
pub trait Operations: Sized {
    /// The pointer type the driver data is stored in, e.g. `KBox<Self>` or `Arc<Self>`.
    type Ptr: ForeignOwnable + Send + Sync;

    /// Called when this RNG becomes the current RNG of the hwrng core, before the first `read`.
    #[optional]
    fn init(_data: <Self::Ptr as ForeignOwnable>::Borrowed<'_>) -> Result;

    /// Called when this RNG stops being the current one, including when it is unregistered.
    #[optional]
    fn cleanup(_data: <Self::Ptr as ForeignOwnable>::Borrowed<'_>);

    /// Fill `buffer` with random bytes and return how many were written.
    ///
    /// Returning `Ok(0)` means no data is available right now. Returning more than
    /// `buffer.len()` is a bug: the core then gets `EINVAL` instead of the data. `wait` says
    /// whether the caller may sleep until data is available.
    fn read(
        data: <Self::Ptr as ForeignOwnable>::Borrowed<'_>,
        buffer: &mut [u8],
        wait: bool,
    ) -> Result<usize>;
}

/// The C callbacks that forward to an [`Operations`] implementation.
struct HwRngVTable<T: Operations>(PhantomData<T>);

impl<T: Operations> HwRngVTable<T> {
    /// # Safety
    ///
    /// `rng` must be the `struct hwrng` of a live [`HwRngRegistration<T>`].
    unsafe extern "C" fn init(rng: *mut bindings::hwrng) -> c_int {
        // SAFETY: By the safety requirements and the invariants of `HwRngRegistration`, `priv`
        // holds driver data from `into_foreign()` that is not freed while a callback runs.
        let data = unsafe { T::Ptr::borrow((*rng).priv_ as *mut c_void) };
        match T::init(data) {
            Ok(()) => 0,
            Err(err) => err.to_errno(),
        }
    }

    /// # Safety
    ///
    /// `rng` must be the `struct hwrng` of a live [`HwRngRegistration<T>`].
    unsafe extern "C" fn cleanup(rng: *mut bindings::hwrng) {
        // SAFETY: As in `init`.
        let data = unsafe { T::Ptr::borrow((*rng).priv_ as *mut c_void) };
        T::cleanup(data);
    }

    /// # Safety
    ///
    /// `rng` must be the `struct hwrng` of a live [`HwRngRegistration<T>`], and `buf` must be
    /// valid for writes of `max` bytes and not otherwise accessed during the call.
    unsafe extern "C" fn read(
        rng: *mut bindings::hwrng,
        buf: *mut c_void,
        max: usize,
        wait: bool,
    ) -> c_int {
        // SAFETY: As in `init`.
        let data = unsafe { T::Ptr::borrow((*rng).priv_ as *mut c_void) };

        // SAFETY: By the safety requirements, `buf` is valid for writes of `max` bytes and
        // exclusively ours for the duration of the call.
        let buffer = unsafe { core::slice::from_raw_parts_mut(buf.cast::<u8>(), max) };

        match T::read(data, buffer, wait) {
            // The core copies the returned number of bytes out of its buffer, so a count larger
            // than the buffer must never reach it.
            Ok(n) if n <= max => c_int::try_from(n).unwrap_or(EINVAL.to_errno()),
            Ok(_) => EINVAL.to_errno(),
            Err(err) => err.to_errno(),
        }
    }
}
