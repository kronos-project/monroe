use crate::hook::{SenderHook, Spinlock};
use std::{
    any::Any,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Waker},
};

pub trait Signal: Send + Sync + 'static {
    fn fire(&self);
    fn as_any(&self) -> &(dyn Any + 'static);
    fn as_ptr(&self) -> *const ();
}

pub struct AsyncSignal {
    waker: Spinlock<Waker>,
    woken: AtomicBool,
}

impl AsyncSignal {
    pub fn new(cx: &Context) -> Self {
        AsyncSignal {
            waker: Spinlock::new(cx.waker().clone()),
            woken: AtomicBool::new(false),
        }
    }
}

impl Signal for AsyncSignal {
    fn fire(&self) {
        self.woken.store(true, Ordering::SeqCst);
        self.waker.lock().wake_by_ref();
    }

    fn as_any(&self) -> &(dyn Any + 'static) {
        self
    }
    fn as_ptr(&self) -> *const () {
        self as *const _ as *const ()
    }
}

impl<T> SenderHook<T, AsyncSignal> {
    pub fn update_waker(&self, cx_waker: &Waker) {
        if !self.signal.waker.lock().will_wake(cx_waker) {
            *self.signal.waker.lock() = cx_waker.clone();

            // Avoid the edge case where the waker was woken just before the wakers were
            // swapped.
            if self.signal.woken.load(Ordering::SeqCst) {
                cx_waker.wake_by_ref();
            }
        }
    }
}
