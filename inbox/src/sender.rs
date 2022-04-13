use crate::{
    hook::SenderHook,
    signal::{AsyncSignal, Signal},
    Shared,
};
use futures_core::FusedFuture;
use pin_project_lite::pin_project;
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{atomic::Ordering, Arc, Weak},
    task::Poll,
};

/// An error that may be emitted when attempting to send a value into a channel on a sender when
/// all receivers are dropped.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct SendError<T>(pub T);

impl<T> SendError<T> {
    /// Consume the error, yielding the message that failed to send.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "SendError(..)".fmt(f)
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "sending on a closed channel".fmt(f)
    }
}

impl<T> std::error::Error for SendError<T> {}

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

/// The transmitting end of a channel.
pub struct Sender<T> {
    pub(crate) shared: Arc<Shared<T>>,
}

impl<T> Sender<T> {
    /// Asynchronously send a value into the channel, returning
    /// an error if all receivers have been dropped. If the
    /// channel is bounded and is full, the returned future will
    /// yield to the async runtime.
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
            shared: self.shared.clone(),
        }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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

pin_project! {
    /// A future that sends a value into a channel.
    ///
    /// Can be created via [`Sender::send_async`] or
    /// [`Sender::into_send_async`].
    #[must_use = "futures/streams/sinks do nothing unless you `.await` or poll them"]
    pub struct SendFut<'a, T> {
        sender: &'a Sender<T>,
        // only none after dropping
        hook: Option<SendState<T>>,
    }

    impl<T> PinnedDrop for SendFut<'_, T> {
        fn drop(mut this: Pin<&mut Self>) {
            this.reset_hook();
        }
    }
}

impl<T> SendFut<'_, T> {
    /// Reset the hook, clearing it and removing it from the
    /// waiting sender's queue.  This is called on drop.
    fn reset_hook(&mut self) {
        let hook = match self.hook.take() {
            Some(SendState::QueuedItem(hook)) => hook,
            _ => return,
        };

        let mut chan = self.sender.shared.chan.lock();

        // this can't be `None`, because `QueuedItem` state can
        // only exist if we have to wait for free capacity
        let (_, sending) = chan.sending.as_mut().unwrap();

        // remove all waiting signals that are ours
        let our_signal = hook.signal().as_ptr();
        sending.retain(|s| s.signal().as_ptr() != our_signal);
    }
}

impl<T> Future for SendFut<'_, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        // if the state of this future is `QueuedItem`, it means,
        // that this is the second poll, and we were probably
        // woken up by the executor
        if let Some(SendState::QueuedItem(hook)) = self.hook.as_ref() {
            // if no item in our hook, item was send successfully
            if hook.is_empty() {
                return Poll::Ready(Ok(()));
            }

            // on disconnected channel, take item out of hook
            // and return an error
            if self.sender.shared.is_disconnected() {
                return match self.hook.take().unwrap() {
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
        if matches!(self.hook, Some(SendState::NotYetSent(_))) {
            let mut this = self.project();

            let shared = &this.sender.shared;
            let this_hook = &mut this.hook;

            let item = match this_hook.take().unwrap() {
                SendState::NotYetSent(item) => item,
                SendState::QueuedItem(_) => unreachable!(),
            };

            return shared.send(item, |item, mut chan| {
                let hook = SenderHook::new(Some(item), AsyncSignal::new(cx));
                chan.sending.as_mut().unwrap().1.push_back(hook.clone());
                drop(chan);

                // when we "block"/wait, we update our state so
                // when we get woken up again, we jump into the
                // first `if` clause and try to pop the item from
                // the queue
                **this_hook = Some(SendState::QueuedItem(hook));
                Poll::Pending
            });
        }

        // unreachable
        Poll::Pending
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
