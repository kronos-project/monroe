use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{atomic::Ordering, Arc},
    task::{Context, Poll},
};

use futures_core::FusedFuture;

use crate::{
    hook::ReceiverHook,
    signal::{AsyncSignal, Signal},
    Sender, Shared,
};

/// Error produced by the receiver when waiting to receive a value
/// fails due to all senders being disconnected.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RecvError {
    /// All senders were dropped and no messages are waiting in the
    /// channel, so no further messages can be received.
    Disconnected,
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvError::Disconnected => "receiving on a closed channel".fmt(f),
        }
    }
}

impl std::error::Error for RecvError {}

/// Error produced by the receiver when trying to wait for a value
/// fails due to all senders being disconnected.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TryRecvError {
    /// This channel is currently empty, but the Sender(s) have not
    /// yet disconnected, so data may still become available.
    Empty,
    /// All senders were dropped and no messages are waiting in the
    /// channel, so no further messages can be received.
    Disconnected,
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryRecvError::Empty => "receiving on an empty channel".fmt(f),
            TryRecvError::Disconnected => "receiving on a closed channel".fmt(f),
        }
    }
}

impl std::error::Error for TryRecvError {}

impl From<RecvError> for TryRecvError {
    fn from(err: RecvError) -> Self {
        match err {
            RecvError::Disconnected => TryRecvError::Disconnected,
        }
    }
}

/// The receiving end of a channel.
pub struct Receiver<T> {
    pub(crate) shared: Arc<Shared<T>>,
}

impl<T> Receiver<T> {
    /// Asynchronously receive a value from the channel, returning
    /// an error if all senders have been dropped.
    ///
    /// If the channel is empty, the returned future will yield
    /// to the async runtime.
    pub fn recv(&self) -> RecvFut<'_, T> {
        RecvFut::new(self)
    }

    /// Pulls all pending messages from waiting senders, puts
    /// them into the message queue, and then clears the message
    /// queue, so the channel becomes empty.
    pub fn clear_channel(&self) {
        self.shared.clear_channel()
    }

    /// Creates a new [`Sender`] for this receiver.
    pub fn create_sender(&self) -> Sender<T> {
        self.shared.sender_count.fetch_add(1, Ordering::Relaxed);
        Sender {
            shared: self.shared.clone(),
        }
    }

    /// Returns true if all [`Sender`]s for this channel have been
    /// dropped.
    pub fn is_disconnected(&self) -> bool {
        self.shared.is_disconnected()
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // this is the only receiver, so disconnect channel
        self.shared.disconnect_all();
    }
}

/// A future which allows asynchronously receiving a message.
///
/// Can be created via [`Receiver::recv`].
#[must_use = "futures/streams/sinks do nothing unless you `.await` or poll them"]
pub struct RecvFut<'a, T> {
    receiver: &'a Receiver<T>,
    hook: Option<Arc<ReceiverHook<T, AsyncSignal>>>,
}

impl<'a, T> RecvFut<'a, T> {
    fn new(receiver: &'a Receiver<T>) -> Self {
        Self {
            receiver,
            hook: None,
        }
    }

    /// Resets the hook, clearing it and removing it from the
    /// waiting receivers queue,
    ///
    /// Wakes another receiver if this receiver has been woken,
    /// so as not to cause any missed wakeups.
    ///
    /// This is called on drop.
    fn reset_hook(&mut self) {
        let hook = match self.hook.take() {
            Some(hook) => hook,
            None => return,
        };

        let mut chan = self.receiver.shared.chan.lock();

        // remove all references of our hook from the waiting list
        let our_hook = hook.signal().as_ptr();
        chan.waiting.retain(|s| s.signal().as_ptr() != our_hook);

        // if we we're woken up, wake up another waiting receiver so we don't miss a wake up
        if hook.signal().woken.load(Ordering::SeqCst) && !chan.queue.is_empty() {
            if let Some(s) = chan.waiting.pop_front() {
                s.signal().fire();
            }
        }
    }
}

impl<'a, T> Drop for RecvFut<'a, T> {
    fn drop(&mut self) {
        self.reset_hook();
    }
}

impl<'a, T> Future for RecvFut<'a, T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // if hook is some and we we're most likely woken up by a sender
        // to indicate that there's a message ready in the queue
        if self.hook.is_some() {
            // there is a message available, so return it
            if let Ok(msg) = self.receiver.shared.try_recv() {
                return Poll::Ready(Ok(msg));
            }

            // channel is disconnected
            if self.receiver.shared.is_disconnected() {
                return Poll::Ready(Err(RecvError::Disconnected));
            }

            // otherwise, we update the waker and wait again
            let hook = self.hook.as_ref().map(Arc::clone).unwrap();
            hook.update_waker(cx.waker());

            self.receiver.shared.chan.lock().waiting.push_back(hook);

            // to avoid a missed wakeup, re-check disconnect status here because the channel
            // might have gotten shut down before we had a chance to push our hook
            return if self.receiver.shared.is_disconnected() {
                // and now, to avoid a race condition between the first recv attempt and the
                // disconnect check we just performed, attempt to recv again just in case we
                // missed something.
                Poll::Ready(
                    self.receiver
                        .shared
                        .try_recv()
                        .map(Ok)
                        .unwrap_or(Err(RecvError::Disconnected)),
                )
            } else {
                Poll::Pending
            };
        }

        let this = self.get_mut();
        let shared = &this.receiver.shared;
        let this_hook = &mut this.hook;

        shared.recv(|mut chan| {
            // we wait by putting our signal hook into the waiting list,
            // and updating our internal state
            let hook = ReceiverHook::new(AsyncSignal::new(cx));
            chan.waiting.push_back(hook.clone());
            drop(chan);

            *this_hook = Some(hook);
            Poll::Pending
        })
    }
}

impl<'a, T> FusedFuture for RecvFut<'a, T> {
    fn is_terminated(&self) -> bool {
        self.receiver.shared.is_disconnected() && self.receiver.shared.is_empty()
    }
}
