// SPDX-License-Identifier: GPL-2.0

//! Completion support.
//!
//! Reference: <https://docs.kernel.org/scheduler/completion.html>
//!
//! C header: [`include/linux/completion.h`](srctree/include/linux/completion.h)

use crate::{
    bindings,
    prelude::*,
    time::{Delta, Jiffy},
    types::Opaque,
};

/// Synchronization primitive to signal when a certain task has been completed.
///
/// The [`Completion`] synchronization primitive signals when a certain task has been completed by
/// waking up other tasks that have been queued up to wait for the [`Completion`] to be completed.
///
/// # Examples
///
/// ```
/// use kernel::sync::{Arc, Completion};
/// use kernel::workqueue::{self, impl_has_work, new_work, Work, WorkItem};
///
/// #[pin_data]
/// struct MyTask {
///     #[pin]
///     work: Work<MyTask>,
///     #[pin]
///     done: Completion,
/// }
///
/// impl_has_work! {
///     impl HasWork<Self> for MyTask { self.work }
/// }
///
/// impl MyTask {
///     fn new() -> Result<Arc<Self>> {
///         let this = Arc::pin_init(pin_init!(MyTask {
///             work <- new_work!("MyTask::work"),
///             done <- Completion::new(),
///         }), GFP_KERNEL)?;
///
///         let _ = workqueue::system().enqueue(this.clone());
///
///         Ok(this)
///     }
///
///     fn wait_for_completion(&self) {
///         self.done.wait_for_completion();
///
///         pr_info!("Completion: task complete\n");
///     }
/// }
///
/// impl WorkItem for MyTask {
///     type Pointer = Arc<MyTask>;
///
///     fn run(this: Arc<MyTask>) {
///         // process this task
///         this.done.complete_all();
///     }
/// }
///
/// let task = MyTask::new()?;
/// task.wait_for_completion();
/// # Ok::<(), Error>(())
/// ```
#[pin_data]
pub struct Completion {
    #[pin]
    inner: Opaque<bindings::completion>,
}

// SAFETY: `Completion` is safe to be send to any task.
unsafe impl Send for Completion {}

// SAFETY: `Completion` is safe to be accessed concurrently.
unsafe impl Sync for Completion {}

impl Completion {
    /// Create an initializer for a new [`Completion`].
    pub fn new() -> impl PinInit<Self> {
        pin_init!(Self {
            inner <- Opaque::ffi_init(|slot: *mut bindings::completion| {
                // SAFETY: `slot` is a valid pointer to an uninitialized `struct completion`.
                unsafe { bindings::init_completion(slot) };
            }),
        })
    }

    fn as_raw(&self) -> *mut bindings::completion {
        self.inner.get()
    }

    /// Signal all tasks waiting on this completion.
    ///
    /// This method wakes up all tasks waiting on this completion; after this operation the
    /// completion is permanently done, i.e. signals all current and future waiters.
    #[inline]
    pub fn complete_all(&self) {
        // SAFETY: `self.as_raw()` is a pointer to a valid `struct completion`.
        unsafe { bindings::complete_all(self.as_raw()) };
    }

    /// Wait for completion of a task.
    ///
    /// This method waits for the completion of a task; it is not interruptible and there is no
    /// timeout.
    ///
    /// See also [`Completion::complete_all`].
    #[inline]
    pub fn wait_for_completion(&self) {
        // SAFETY: `self.as_raw()` is a pointer to a valid `struct completion`.
        unsafe { bindings::wait_for_completion(self.as_raw()) };
    }

    /// Wait for completion of a task with a timeout.
    ///
    /// This method waits for the completion of a task; it is not interruptible and waits for at
    /// most `timeout`, which must be non-negative (a negative or zero span times out immediately
    /// unless the task is already complete).
    ///
    /// Returns the remaining time, in jiffies, after the wait on success, or
    /// [`ETIMEDOUT`] on timeout.
    ///
    /// See also [`Completion::complete_all`].
    ///
    /// # Examples
    ///
    /// This example waits for a task with a timeout; the task never completes, so the wait times
    /// out:
    ///
    /// ```
    /// use kernel::sync::{Arc, Completion};
    /// use kernel::time::Delta;
    ///
    /// #[pin_data]
    /// struct MyTask {
    ///     #[pin]
    ///     done: Completion,
    /// }
    ///
    /// let task = Arc::pin_init(pin_init!(MyTask {
    ///     done <- Completion::new(),
    /// }), GFP_KERNEL)?;
    ///
    /// // `done` is never completed, so the wait times out after 10ms:
    /// assert_eq!(
    ///     task.done.wait_for_completion_timeout(Delta::from_millis(10)),
    ///     Err(ETIMEDOUT),
    /// );
    ///
    /// # Ok::<(), Error>(())
    /// ```
    #[inline]
    pub fn wait_for_completion_timeout(&self, timeout: Delta) -> Result<Delta<Jiffy>> {
        // SAFETY: `self.as_raw()` is a pointer to a valid `struct completion`, and
        // `to_jiffies_timeout()` bounds the converted span for the C API.
        let remaining = unsafe {
            bindings::wait_for_completion_timeout(
                self.as_raw(),
                timeout.to_jiffies_timeout().as_jiffies_unsigned(),
            )
        };

        if remaining == 0 {
            Err(ETIMEDOUT)
        } else {
            // CAST: `remaining` is positive and bounded by the C API to at most the timeout in
            // jiffies, which `to_jiffies_timeout()` caps at `MAX_JIFFY_OFFSET`, so it fits in
            // an `isize`.
            Ok(Delta::from_jiffies(remaining as isize))
        }
    }

    /// Wait for completion of a task with a timeout, interruptible by a `SIGKILL` signal.
    ///
    /// This method waits for the completion of a task; it can be interrupted by a kill signal
    /// and waits for at most `timeout`, which must be non-negative (a negative or zero span times
    /// out immediately unless the task is already complete).
    ///
    /// Returns the remaining time, in jiffies, after the wait on success, [`ETIMEDOUT`] on
    /// timeout, or [`ERESTARTSYS`] if interrupted by a kill signal.
    #[inline]
    pub fn wait_for_completion_killable_timeout(&self, timeout: Delta) -> Result<Delta<Jiffy>> {
        // Interrupted by a kill signal: the C API reports this as `-ERESTARTSYS`.
        const KILLED: isize = -(bindings::ERESTARTSYS as isize);

        // SAFETY: `self.as_raw()` is a pointer to a valid `struct completion`, and
        // `to_jiffies_timeout()` bounds the converted span for the C API.
        let remaining = unsafe {
            bindings::wait_for_completion_killable_timeout(
                self.as_raw(),
                timeout.to_jiffies_timeout().as_jiffies_unsigned(),
            )
        };

        match remaining {
            KILLED => Err(ERESTARTSYS),
            0 => Err(ETIMEDOUT),
            // `remaining` is positive and, as per the C API, bounded by the timeout in jiffies,
            // which `to_jiffies_timeout()` caps at `MAX_JIFFY_OFFSET`, so it fits in an `isize`.
            n => Ok(Delta::from_jiffies(n)),
        }
    }
}
