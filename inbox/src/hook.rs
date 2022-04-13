use crate::signal::AsyncSignal;
use std::{
    marker::PhantomData,
    sync::{atomic::Ordering, Arc},
    task::Waker,
};

pub type Spinlock<T> = spin::Mutex<T>;

/// A [`SenderHook`] represents the signal that a waiting sender
/// will insert into the `sending` list.
pub struct SenderHook<T, S: ?Sized> {
    pub(crate) slot: Spinlock<Option<T>>,
    pub(crate) signal: S,
}

impl<T, S: ?Sized> SenderHook<T, S> {
    pub fn new(msg: Option<T>, signal: S) -> Arc<Self>
    where
        S: Sized,
    {
        Arc::new(Self {
            slot: Spinlock::new(msg),
            signal,
        })
    }

    pub fn signal(&self) -> &S {
        &self.signal
    }

    pub fn take_msg(&self) -> (T, &S) {
        let msg = self.slot.lock().take().unwrap();
        (msg, self.signal())
    }

    pub fn is_empty(&self) -> bool {
        self.slot.lock().is_none()
    }

    pub fn try_take(&self) -> Option<T> {
        self.slot.lock().take()
    }
}

/// A [`ReceiverHook`] represents the signal that a waiting
/// receiver will insert into the `waiting` list.
pub struct ReceiverHook<T, S: ?Sized> {
    // Use `Spinlock<T>` to keep Send and Sync
    // bounds analog to flume
    _marker: PhantomData<Spinlock<T>>,
    signal: S,
}

impl<T, S: ?Sized> ReceiverHook<T, S> {
    pub fn new(signal: S) -> Arc<Self>
    where
        S: Sized,
    {
        Arc::new(Self {
            signal,
            _marker: PhantomData,
        })
    }

    pub fn signal(&self) -> &S {
        &self.signal
    }
}

impl<T> ReceiverHook<T, AsyncSignal> {
    pub fn update_waker(&self, cx_waker: &Waker) {
        if !self.signal.waker.lock().will_wake(cx_waker) {
            *self.signal.waker.lock() = cx_waker.clone();

            // avoid the edge case where the waker was woken just
            // before the wakers were swapped.
            if self.signal.woken.load(Ordering::SeqCst) {
                cx_waker.wake_by_ref();
            }
        }
    }
}
