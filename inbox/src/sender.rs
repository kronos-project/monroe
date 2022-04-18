use crate::{
    hook::SenderHook,
    signal::{AsyncSignal, Signal},
    Chan, Shared,
};
use futures_core::{ready, FusedFuture};
use parking_lot::MutexGuard;
use pin_project_lite::pin_project;
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{atomic::Ordering, Arc, Weak},
    task::{self, Poll},
    time::Duration,
};
use tokio::time::Sleep;

/// Error produced by the sender when trying to send a value to
/// an already dropped receiver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendError<T>(pub T);

impl<T> SendError<T> {
    /// Consume the error, yielding the message that failed to
    /// be sent.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "sending on a closed channel".fmt(f)
    }
}

impl<T: fmt::Debug> std::error::Error for SendError<T> {}

/// An error that may be emitted when sending a value into
/// a channel on a sender with a timeout when the send
/// operation times out or all receivers are dropped.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SendTimeoutError<T> {
    /// A timeout occurred when attempting to send the message.
    Timeout(T),
    /// All channel receivers were dropped and so the message
    /// has nobody to receive it.
    Disconnected(T),
}

impl<T> fmt::Display for SendTimeoutError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendTimeoutError::Timeout(_) => f.write_str("timed out sending on a full channel"),
            SendTimeoutError::Disconnected(_) => f.write_str("sending on a closed channel"),
        }
    }
}

impl<T: fmt::Debug> std::error::Error for SendTimeoutError<T> {}

impl<T> From<SendError<T>> for SendTimeoutError<T> {
    fn from(SendError(msg): SendError<T>) -> Self {
        SendTimeoutError::Disconnected(msg)
    }
}

/// An error that may be emitted when attempting to send a
/// value into a channel on a sender when the channel
/// is full or all receivers are dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrySendError<T> {
    /// The channel the message is sent on has a finite capacity
    /// and was full when the send was attempted.
    Full(T),
    /// All channel receivers were dropped and so the message
    /// has nobody to receive it.
    Disconnected(T),
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrySendError::Full(_) => f.write_str("sending on a full channel"),
            TrySendError::Disconnected(_) => f.write_str("sending on a closed channel"),
        }
    }
}

impl<T: fmt::Debug> std::error::Error for TrySendError<T> {}

impl<T> From<SendError<T>> for TrySendError<T> {
    fn from(SendError(msg): SendError<T>) -> Self {
        TrySendError::Disconnected(msg)
    }
}

/// The transmitting end of a channel.
pub struct Sender<T> {
    pub(crate) shared: Arc<Shared<T>>,
}

impl<T> Sender<T> {
    /// Asynchronously send a value into the channel, returning
    /// an error if all receivers have been dropped.
    ///
    /// If the channel is bounded and is full,
    /// the returned future will yield to the async runtime.
    ///
    /// In the current implementation, the returned future will
    /// not yield to the async runtime if the channel is
    /// unbounded. This may change in later versions.
    pub fn send(&self, msg: T) -> SendFut<'_, T> {
        SendFut {
            sender: self,
            hook: Some(SendState::NotYetSent(msg)),
        }
    }

    /// Asynchronously send a value into the channel, returning
    /// an error if all receivers have been dropped, or the
    /// timeout has expired.
    ///
    /// If the channel is bounded and is full,
    /// the returned future will yield to the async runtime.
    ///
    /// In the current implementation, the returned future will
    /// not yield to the async runtime if the channel is
    /// unbounded. This may change in later versions.
    pub fn send_timeout(&self, msg: T, timeout: Duration) -> SendTimeoutFut<'_, T> {
        SendTimeoutFut {
            fut: SendFut {
                sender: self,
                hook: Some(SendState::NotYetSent(msg)),
            },
            timeout: tokio::time::sleep(timeout),
        }
    }

    /// Attempt to send a value into the channel.
    ///
    /// If the channel is bounded and full, or all receivers
    /// have been dropped, an error is returned.
    ///
    /// If the channel associated with this sender is unbounded,
    /// this method has the same behaviour as Sender::send.
    pub fn try_send(&self, msg: T) -> Result<(), TrySendError<T>> {
        self.shared.try_send(msg)
    }

    /// Creates a new [`WeakSender`] for this channel.
    pub fn downgrade(&self) -> WeakSender<T> {
        WeakSender {
            shared: Arc::downgrade(&self.shared),
        }
    }

    /// Returns true if all receivers for this channel have been dropped.
    pub fn is_disconnected(&self) -> bool {
        self.shared.is_disconnected()
    }
}

impl<T> Clone for Sender<T> {
    /// Clone this sender. [`Sender`] acts as a handle to the
    /// ending a channel. Remaining channel contents will only be
    /// cleaned up when all senders and the receiver have been
    /// dropped.
    fn clone(&self) -> Self {
        self.shared.sender_count.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender").finish()
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // Notify receivers that all senders have been dropped if
        // the number of senders drops to 0.
        if self.shared.sender_count.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.shared.disconnect_all();
        }
    }
}

/// [`WeakSender`] is a version of a [`Sender`] that won't count
/// into the channels sender count.
///
/// Thus, if for a channel there only exist [`WeakSender`]s, it
/// will be disconnected. Similar to the nature of [`Arc`] and
/// [`Weak`].
///
/// To use a [`WeakSender`], it first has to be upgraded into a
/// [`Sender`] using the [`WeakSender::upgrade`] method.
pub struct WeakSender<T> {
    shared: Weak<Shared<T>>,
}

impl<T> WeakSender<T> {
    /// Attempts to upgrade this [`WeakSender`] into a [`Sender`],
    /// which can then be used to send messages into this channel.
    ///
    /// This method might fail (return [`None`]),
    /// if the channel was already dropped.
    pub fn upgrade(&self) -> Option<Sender<T>> {
        self.shared.upgrade().map(|shared| {
            shared.sender_count.fetch_add(1, Ordering::Relaxed);
            Sender { shared }
        })
    }
}

impl<T> Clone for WeakSender<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Weak::clone(&self.shared),
        }
    }
}

impl<T> fmt::Debug for WeakSender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakSender").finish()
    }
}

pin_project! {
    /// A future that sends a value into a channel and may time.
    ///
    /// Can be created via [`Sender::send_timeout`].
    #[must_use = "futures/streams/sinks do nothing unless you `.await` or poll them"]
    pub struct SendTimeoutFut<'a, T> {
        #[pin]
        fut: SendFut<'a, T>,
        #[pin]
        timeout: Sleep,
    }
}

impl<T> Future for SendTimeoutFut<'_, T> {
    type Output = Result<(), SendTimeoutError<T>>;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        match this.timeout.poll(cx) {
            // timed out
            Poll::Ready(_) => {
                let item = match this.fut.hook.take().unwrap() {
                    SendState::NotYetSent(item) => item,
                    // if the item was queued, reset the hook and
                    // take the item out of the hook
                    SendState::QueuedItem(hook) => {
                        let mut chan = this.fut.sender.shared.chan.lock();
                        SendFut::reset_hook(&mut chan, hook.signal().as_ptr());
                        hook.try_take().unwrap()
                    }
                };

                Poll::Ready(Err(SendTimeoutError::Timeout(item)))
            }
            Poll::Pending => Poll::Ready(ready!(this.fut.poll(cx)).map_err(Into::into)),
        }
    }
}

impl<T> FusedFuture for SendTimeoutFut<'_, T> {
    fn is_terminated(&self) -> bool {
        self.fut.sender.shared.is_disconnected() || self.timeout.is_elapsed()
    }
}

pin_project! {
    /// A future that sends a value into a channel.
    ///
    /// Can be created via [`Sender::send`].
    #[must_use = "futures/streams/sinks do nothing unless you `.await` or poll them"]
    pub struct SendFut<'a, T> {
        sender: &'a Sender<T>,
        // only none after dropping
        hook: Option<SendState<T>>,
    }

    impl<T> PinnedDrop for SendFut<'_, T> {
        fn drop(mut this: Pin<&mut Self>) {
            let this = this.project();

            if let Some(SendState::QueuedItem(hook)) = this.hook.take() {
                let mut chan = this.sender.shared.chan.lock();
                SendFut::reset_hook(&mut chan, hook.signal().as_ptr());
            }
        }
    }
}

impl<'a, T> SendFut<'a, T> {
    /// Reset the hook, clearing it and removing it from the
    /// waiting sender's queue.  This is called on drop.
    fn reset_hook(chan: &mut MutexGuard<'_, Chan<T>>, our_signal: *const ()) {
        // this can't be `None`, because `QueuedItem` state can
        // only exist if we have to wait for free capacity
        let (_, sending) = chan.sending.as_mut().unwrap();

        // remove all waiting signals that are ours
        sending.retain(|s| s.signal().as_ptr() != our_signal);
    }
}

impl<T> Future for SendFut<'_, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // if the state of this future is `QueuedItem`, it means,
        // that this is the second poll, and we were probably
        // woken up by the executor
        if let Some(SendState::QueuedItem(hook)) = this.hook.as_ref() {
            // if no item in our hook, item was send successfully
            if hook.is_empty() {
                return Poll::Ready(Ok(()));
            }

            // on disconnected channel, take item out of hook
            // and return an error
            if this.sender.shared.is_disconnected() {
                return match this.hook.take().unwrap() {
                    // this should be unreachable, but we just play safe here
                    SendState::NotYetSent(item) => Poll::Ready(Err(SendError(item))),
                    SendState::QueuedItem(hook) => match hook.try_take() {
                        Some(item) => Poll::Ready(Err(SendError(item))),
                        // this should be unreachable too, but we just play safe again
                        None => Poll::Ready(Ok(())),
                    },
                };
            }

            // we got polled, but the item was not send,
            // so we pend again
            hook.update_waker(cx.waker());
            return Poll::Pending;
        }

        // first time polling, try to send the message using `shared.send`
        if let Some(SendState::NotYetSent(item)) = this.hook.take() {
            return this.sender.shared.send(item, |item, mut chan| {
                let hook = SenderHook::new(Some(item), AsyncSignal::new(cx));
                chan.sending.as_mut().unwrap().1.push_back(hook.clone());
                drop(chan);

                // when we "block"/wait, we update our state so
                // when we get woken up again, we jump into the
                // first `if` clause and try to pop the item from
                // the queue
                *this.hook = Some(SendState::QueuedItem(hook));
                Poll::Pending
            });
        }

        // unreachable
        unreachable!()
    }
}

impl<T> FusedFuture for SendFut<'_, T> {
    fn is_terminated(&self) -> bool {
        self.sender.shared.is_disconnected()
    }
}

enum SendState<T> {
    NotYetSent(T),
    QueuedItem(Arc<SenderHook<T, AsyncSignal>>),
}
