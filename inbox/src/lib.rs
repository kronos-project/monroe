//! Implementation of a custom mpsc channel that is based on [`flume`].
//!
//! This is mostly a stripped version of [`flume`] with some additional features
//! that we need for monroe.
//!
//! [`flume`]: https://docs.rs/flume
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod hook;
mod signal;

mod sender;
pub use sender::*;

mod receiver;
pub use receiver::*;

use hook::{ReceiverHook, SenderHook};
use parking_lot::Mutex;
use signal::Signal;
use std::{
    collections::VecDeque,
    num::NonZeroUsize,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

pub(crate) type SignalVec<T> = VecDeque<Arc<T>>;

pub(crate) struct Chan<T> {
    // `sending` is true if this is a bounded channel and the first element
    // of the tuple is the cap. The latter element is a List of senders that are waiting
    // for the channel to becompe empty
    sending: Option<(NonZeroUsize, SignalVec<SenderHook<T, dyn Signal>>)>,
    // queue contains all items that are waiting for a receiver to
    // pick them up.
    queue: VecDeque<T>,
    // `waiting` contains all current receivers.
    // The `Hook` inside is responsible for telling the receiver
    // that a message is ready to be taken out of the `queue`
    waiting: SignalVec<ReceiverHook<T, dyn Signal>>,
}

impl<T> Chan<T> {
    /// This method will go through every waiting sender in the `waiting` list,
    /// take their message, fire their signal, and push the message into `queue`, until
    /// the capacity of this channel has reached.
    ///
    /// If `pull_extra` is `true`, the capacity will be increased by one.
    fn pull_pending(&mut self, pull_extra: bool) {
        let (cap, sending) = match &mut self.sending {
            Some(x) => x,
            None => return,
        };

        let effective_cap = cap.get() + pull_extra as usize;

        while self.queue.len() < effective_cap {
            // take one waiting sender, but stop if there are no more
            let signal = match sending.pop_front() {
                Some(s) => s,
                None => break,
            };

            let (msg, signal) = signal.take_msg();
            signal.fire();
            self.queue.push_back(msg);
        }
    }
}

pub(crate) struct Shared<T> {
    chan: Mutex<Chan<T>>,
    disconnected: AtomicBool,
    sender_count: AtomicUsize,
}

impl<T> Shared<T> {
    fn is_disconnected(&self) -> bool {
        self.disconnected.load(Ordering::SeqCst)
    }

    /// Disconnect anything listening on this channel (this will not prevent receivers receiving
    /// msgs that have already been sent)
    fn disconnect_all(&self) {
        self.disconnected.store(true, Ordering::Relaxed);

        let mut chan = self.chan.lock();
        chan.pull_pending(false);

        if let Some((_, sending)) = chan.sending.as_ref() {
            sending.iter().for_each(|hook| {
                hook.signal().fire();
            })
        }

        chan.waiting.iter().for_each(|hook| {
            hook.signal().fire();
        });
    }

    pub fn send<S, R>(
        &self,
        msg: T,
        mk_signal: impl FnOnce(T) -> Arc<SenderHook<T, S>>,
        do_block: impl FnOnce(Arc<SenderHook<T, S>>) -> R,
    ) -> R
    where
        S: Signal,
        R: From<Result<(), SendError<T>>>,
    {
        let mut chan = self.chan.lock();

        // disconnected, there's nothing to send
        if self.is_disconnected() {
            return R::from(Err(SendError(msg)));
        }

        // if there are waiting receivers,
        // try to send message directly to them
        if !chan.waiting.is_empty() {
            let receiver = match chan.waiting.pop_front() {
                Some(r) => r,
                // there is no waiting receiver, so just push message to queue
                // and return successfully
                None => {
                    chan.queue.push_back(msg);
                    return Ok(()).into();
                }
            };

            match receiver.send(msg) {
                // `receiver` has kind `Trigger`
                Some(msg) => {
                    receiver.signal().fire();
                    chan.queue.push_back(msg);
                    drop(chan);
                }
                // `receiver` has kind `Slot`
                None => {
                    drop(chan);
                    receiver.signal().fire();
                }
            };

            return R::from(Ok(()));
        }

        // if there are no waiting receivers,
        // we will insert the message into the queue,
        // as long as the capacity is not reached
        if chan
            .sending
            .as_ref()
            .map_or(true, |(cap, _)| chan.queue.len() < cap.get())
        {
            chan.queue.push_back(msg);
            return R::from(Ok(()));
        }

        // need to wait
        let hook = mk_signal(msg);
        chan.sending.as_mut().unwrap().1.push_back(hook.clone());
        drop(chan);

        do_block(hook)
    }
}

#[cfg(test)]
mod tests {}
