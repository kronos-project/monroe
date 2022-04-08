//! Implementation of a custom mpsc channel that is based on [`flume`].
//!
//! This is mostly a stripped version of [`flume`] with some additional features
//! that we need for monroe.
//!
//! [`flume`]: https://docs.rs/flume

use self::signal::{AsyncSignal, Signal};
use futures_core::FusedFuture;
use pin_project::{pin_project, pinned_drop};
use spin::Mutex as Spinlock;
use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard,
    },
    task::{Context, Poll},
    time::Instant,
};

/// Create a channel with no maximum capacity.
///
/// Create an unbounded channel with a [`Sender`] and [`Receiver`] connected to each end respectively. Values sent in
/// one end of the channel will be received on the other end. The channel is thread-safe, and both [`Sender`] and
/// [`Receiver`] may be sent to or shared between threads as necessary. In addition, both [`Sender`] and [`Receiver`]
/// may be cloned.
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared::new(None));
    (
        Sender {
            shared: shared.clone(),
        },
        Receiver { shared },
    )
}

/// Create a channel with a maximum capacity.
///
/// Create a bounded channel with a [`Sender`] and [`Receiver`] connected to each end respectively. Values sent in one
/// end of the channel will be received on the other end. The channel is thread-safe, and both [`Sender`] and
/// [`Receiver`] may be sent to or shared between threads as necessary. In addition, both [`Sender`] and [`Receiver`]
/// may be cloned.
///
/// Unlike an [`unbounded`] channel, if there is no space left for new messages, calls to
/// [`Sender::send`] will block (unblocking once a receiver has made space). If blocking behaviour
/// is not desired, [`Sender::try_send`] may be used.
///
/// Like `std::sync::mpsc`, `flume` supports 'rendezvous' channels. A bounded queue with a maximum capacity of zero
/// will block senders until a receiver is available to take the value. You can imagine a rendezvous channel as a
/// ['Glienicke Bridge'](https://en.wikipedia.org/wiki/Glienicke_Bridge)-style location at which senders and receivers
/// perform a handshake and transfer ownership of a value.
pub fn bounded<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared::new(Some(cap)));
    (
        Sender {
            shared: shared.clone(),
        },
        Receiver { shared },
    )
}

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

/// A transmitting end of a channel.
pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Sender<T> {
    /// Asynchronously send a value into the channel, returning an error if all receivers have been
    /// dropped. If the channel is bounded and is full, the returned future will yield to the async
    /// runtime.
    ///
    /// In the current implementation, the returned future will not yield to the async runtime if the
    /// channel is unbounded. This may change in later versions.
    pub fn send(&self, msg: T) -> SendFut<'_, T> {
        SendFut {
            sender: self,
            hook: Some(SendState::NotYetSent(msg)),
        }
    }
}

impl<T> Clone for Sender<T> {
    /// Clone this sender. [`Sender`] acts as a handle to the ending a channel. Remaining channel
    /// contents will only be cleaned up when all senders and the receiver have been dropped.
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
        // Notify receivers that all senders have been dropped if the number of senders drops to 0.
        if self.shared.sender_count.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.shared.disconnect_all();
        }
    }
}

/// A future that sends a value into a channel.
///
/// Can be created via [`Sender::send_async`] or [`Sender::into_send_async`].
#[must_use = "futures/streams/sinks do nothing unless you `.await` or poll them"]
#[pin_project(PinnedDrop)] // FIXME: maybe remove `pin-project` dependency
pub struct SendFut<'a, T> {
    sender: &'a Sender<T>,
    // Only none after dropping
    hook: Option<SendState<T>>,
}

impl<'a, T> SendFut<'a, T> {
    /// Reset the hook, clearing it and removing it from the waiting sender's queue. This is called
    /// on drop and just before `start_send` in the `Sink` implementation.
    fn reset_hook(&mut self) {
        if let Some(SendState::QueuedItem(hook)) = self.hook.take() {
            let hook: Arc<Hook<T, dyn Signal>> = hook;
            wait_lock(&self.sender.shared.chan)
                .sending
                .as_mut()
                .unwrap()
                .1
                .retain(|s| s.signal().as_ptr() != hook.signal().as_ptr());
        }
    }
}

#[allow(clippy::needless_lifetimes)]
#[pinned_drop]
impl<'a, T> PinnedDrop for SendFut<'a, T> {
    fn drop(mut self: Pin<&mut Self>) {
        self.reset_hook()
    }
}

impl<'a, T> Future for SendFut<'a, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(SendState::QueuedItem(hook)) = self.hook.as_ref() {
            if hook.is_empty() {
                Poll::Ready(Ok(()))
            } else if self.sender.shared.is_disconnected() {
                match self.hook.take().unwrap() {
                    SendState::NotYetSent(item) => Poll::Ready(Err(SendError(item))),
                    SendState::QueuedItem(hook) => match hook.try_take() {
                        Some(item) => Poll::Ready(Err(SendError(item))),
                        None => Poll::Ready(Ok(())),
                    },
                }
            } else {
                hook.update_waker(cx.waker());
                Poll::Pending
            }
        } else if let Some(SendState::NotYetSent(_)) = self.hook {
            let mut mut_self = self.project();
            let (shared, this_hook) = (&mut_self.sender.shared, &mut mut_self.hook);

            shared.send(
                // item
                match this_hook.take().unwrap() {
                    SendState::NotYetSent(item) => item,
                    SendState::QueuedItem(_) => return Poll::Ready(Ok(())),
                },
                // should_block
                true,
                // make_signal
                |msg| Hook::slot(Some(msg), AsyncSignal::new(cx, false)),
                // do_block
                |hook| {
                    **this_hook = Some(SendState::QueuedItem(hook));
                    Poll::Pending
                },
            )
        } else {
            // Nothing to do
            Poll::Ready(Ok(()))
        }
    }
}

impl<'a, T> FusedFuture for SendFut<'a, T> {
    fn is_terminated(&self) -> bool {
        self.sender.shared.is_disconnected()
    }
}

/// A future which allows asynchronously receiving a message.
///
/// Can be created via [`Receiver::recv_async`] or [`Receiver::into_recv_async`].
#[must_use = "futures/streams/sinks do nothing unless you `.await` or poll them"]
pub struct RecvFut<'a, T> {
    receiver: &'a Receiver<T>,
    hook: Option<Arc<Hook<T, AsyncSignal>>>,
}

impl<'a, T> RecvFut<'a, T> {
    fn new(receiver: &'a Receiver<T>) -> Self {
        Self {
            receiver,
            hook: None,
        }
    }

    /// Reset the hook, clearing it and removing it from the waiting receivers queue and waking
    /// another receiver if this receiver has been woken, so as not to cause any missed wakeups.
    /// This is called on drop and after a new item is received in `Stream::poll_next`.
    fn reset_hook(&mut self) {
        if let Some(hook) = self.hook.take() {
            let hook: Arc<Hook<T, dyn Signal>> = hook;
            let mut chan = wait_lock(&self.receiver.shared.chan);
            // We'd like to use `Arc::ptr_eq` here but it doesn't seem to work consistently with wide pointers?
            chan.waiting
                .retain(|s| s.signal().as_ptr() != hook.signal().as_ptr());
            if hook
                .signal()
                .as_any()
                .downcast_ref::<AsyncSignal>()
                .unwrap()
                .woken
                .load(Ordering::SeqCst)
            {
                // If this signal has been fired, but we're being dropped (and so not listening to it),
                // pass the signal on to another receiver
                chan.try_wake_receiver_if_pending();
            }
        }
    }

    fn poll_inner(
        self: Pin<&mut Self>,
        cx: &mut Context,
        stream: bool,
    ) -> Poll<Result<T, RecvError>> {
        if self.hook.is_some() {
            if let Ok(msg) = self.receiver.shared.recv_sync(None) {
                Poll::Ready(Ok(msg))
            } else if self.receiver.shared.is_disconnected() {
                Poll::Ready(Err(RecvError::Disconnected))
            } else {
                let hook = self.hook.as_ref().map(Arc::clone).unwrap();
                hook.update_waker(cx.waker());
                wait_lock(&self.receiver.shared.chan)
                    .waiting
                    .push_back(hook);
                // To avoid a missed wakeup, re-check disconnect status here because the channel might have
                // gotten shut down before we had a chance to push our hook
                if self.receiver.shared.is_disconnected() {
                    // And now, to avoid a race condition between the first recv attempt and the disconnect check we
                    // just performed, attempt to recv again just in case we missed something.
                    Poll::Ready(
                        self.receiver
                            .shared
                            .recv_sync(None)
                            .map(Ok)
                            .unwrap_or(Err(RecvError::Disconnected)),
                    )
                } else {
                    Poll::Pending
                }
            }
        } else {
            let mut_self = self.get_mut();
            let (shared, this_hook) = (&mut_self.receiver.shared, &mut mut_self.hook);

            shared
                .recv(
                    // should_block
                    true,
                    // make_signal
                    || Hook::trigger(AsyncSignal::new(cx, stream)),
                    // do_block
                    |hook| {
                        *this_hook = Some(hook);
                        Poll::Pending
                    },
                )
                .map(|r| r.map_err(|_| RecvError::Disconnected))
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
        self.poll_inner(cx, false) // stream = false
    }
}

impl<'a, T> FusedFuture for RecvFut<'a, T> {
    fn is_terminated(&self) -> bool {
        self.receiver.shared.is_disconnected() && self.receiver.shared.is_empty()
    }
}

/// An error that may be emitted when attempting to wait for a value on a receiver when all senders
/// are dropped and there are no more messages in the channel.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RecvError {
    /// All senders were dropped and no messages are waiting in the channel, so no further messages can be received.
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

/// The receiving end of a channel.
///
/// Note: Cloning the receiver *does not* turn this channel into a broadcast channel.
/// Each message will only be received by a single receiver. This is useful for
/// implementing work stealing for concurrent programs.
pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Receiver<T> {
    /// Asynchronously receive a value from the channel, returning an error if all senders have been
    /// dropped. If the channel is empty, the returned future will yield to the async runtime.
    pub fn recv(&self) -> RecvFut<'_, T> {
        RecvFut::new(self)
    }

    /// Creates a new [`Sender`] that will send values to this receiver.
    pub fn create_sender(&self) -> Sender<T> {
        self.shared.sender_count.fetch_add(1, Ordering::Relaxed);
        Sender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Clone for Receiver<T> {
    /// Clone this receiver. [`Receiver`] acts as a handle to the ending a channel. Remaining
    /// channel contents will only be cleaned up when all senders and the receiver have been
    /// dropped.
    ///
    /// Note: Cloning the receiver *does not* turn this channel into a broadcast channel.
    /// Each message will only be received by a single receiver. This is useful for
    /// implementing work stealing for concurrent programs.
    fn clone(&self) -> Self {
        self.shared.receiver_count.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Notify senders that all receivers have been dropped if the number of receivers drops
        // to 0.
        if self.shared.receiver_count.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.shared.disconnect_all();
        }
    }
}

// =============== private implementation ============================

enum SendState<T> {
    NotYetSent(T),
    QueuedItem(Arc<Hook<T, AsyncSignal>>),
}

#[inline]
fn wait_lock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap()
}

type SignalVec<T> = VecDeque<Arc<Hook<T, dyn Signal>>>;

struct Hook<T, S: ?Sized>(Option<Spinlock<Option<T>>>, S);

impl<T> Hook<T, signal::SyncSignal> {
    pub fn wait_recv(&self, abort: &AtomicBool) -> Option<T> {
        loop {
            let disconnected = abort.load(Ordering::SeqCst); // Check disconnect *before* msg
            let msg = self.0.as_ref().unwrap().lock().take();
            if let Some(msg) = msg {
                break Some(msg);
            } else if disconnected {
                break None;
            } else {
                self.signal().wait()
            }
        }
    }

    // Err(true) if timeout
    pub fn wait_deadline_recv(&self, abort: &AtomicBool, deadline: Instant) -> Result<T, bool> {
        loop {
            let disconnected = abort.load(Ordering::SeqCst); // Check disconnect *before* msg
            let msg = self.0.as_ref().unwrap().lock().take();
            if let Some(msg) = msg {
                break Ok(msg);
            } else if disconnected {
                break Err(false);
            } else if let Some(dur) = deadline.checked_duration_since(Instant::now()) {
                self.signal().wait_timeout(dur);
            } else {
                break Err(true);
            }
        }
    }
}

impl<T, S: ?Sized + Signal> Hook<T, S> {
    pub fn slot(msg: Option<T>, signal: S) -> Arc<Self>
    where
        S: Sized,
    {
        Arc::new(Self(Some(Spinlock::new(msg)), signal))
    }

    pub fn trigger(signal: S) -> Arc<Self>
    where
        S: Sized,
    {
        Arc::new(Self(None, signal))
    }

    pub fn is_empty(&self) -> bool {
        self.0.as_ref().map(|s| s.lock().is_none()).unwrap_or(true)
    }

    pub fn signal(&self) -> &S {
        &self.1
    }

    pub fn fire_nothing(&self) -> bool {
        self.signal().fire()
    }

    pub fn fire_recv(&self) -> (T, &S) {
        let msg = self.0.as_ref().unwrap().lock().take().unwrap();
        (msg, self.signal())
    }

    pub fn fire_send(&self, msg: T) -> (Option<T>, &S) {
        let ret = match &self.0 {
            Some(hook) => {
                *hook.lock() = Some(msg);
                None
            }
            None => Some(msg),
        };
        (ret, self.signal())
    }

    pub fn try_take(&self) -> Option<T> {
        self.0.as_ref().and_then(|s| s.lock().take())
    }
}

struct Chan<T> {
    sending: Option<(usize, SignalVec<T>)>,
    queue: VecDeque<T>,
    waiting: SignalVec<T>,
}

impl<T> Chan<T> {
    fn pull_pending(&mut self, pull_extra: bool) {
        if let Some((cap, sending)) = &mut self.sending {
            let effective_cap = *cap + pull_extra as usize;

            while self.queue.len() < effective_cap {
                if let Some(s) = sending.pop_front() {
                    let (msg, signal) = s.fire_recv();
                    signal.fire();
                    self.queue.push_back(msg);
                } else {
                    break;
                }
            }
        }
    }

    fn try_wake_receiver_if_pending(&mut self) {
        if !self.queue.is_empty() {
            while Some(false) == self.waiting.pop_front().map(|s| s.fire_nothing()) {}
        }
    }
}

struct Shared<T> {
    chan: Mutex<Chan<T>>,
    disconnected: AtomicBool,
    sender_count: AtomicUsize,
    receiver_count: AtomicUsize,
}

impl<T> Shared<T> {
    fn new(cap: Option<usize>) -> Self {
        Self {
            chan: Mutex::new(Chan {
                sending: cap.map(|cap| (cap, VecDeque::new())),
                queue: VecDeque::new(),
                waiting: VecDeque::new(),
            }),
            disconnected: AtomicBool::new(false),
            sender_count: AtomicUsize::new(1),
            receiver_count: AtomicUsize::new(1),
        }
    }

    fn send<S: Signal, R: From<Result<(), SendError<T>>>>(
        &self,
        msg: T,
        should_block: bool,
        make_signal: impl FnOnce(T) -> Arc<Hook<T, S>>,
        do_block: impl FnOnce(Arc<Hook<T, S>>) -> R,
    ) -> R {
        let mut chan = wait_lock(&self.chan);

        if self.is_disconnected() {
            Err(SendError(msg)).into()
        } else if !chan.waiting.is_empty() {
            let mut msg = Some(msg);

            loop {
                let slot = chan.waiting.pop_front();
                match slot.as_ref().map(|r| r.fire_send(msg.take().unwrap())) {
                    // No more waiting receivers and msg in queue, so break out of the loop
                    None if msg.is_none() => break,
                    // No more waiting receivers, so add msg to queue and break out of the loop
                    None => {
                        chan.queue.push_back(msg.unwrap());
                        break;
                    }
                    Some((Some(m), signal)) => {
                        if signal.fire() {
                            // Was async and a stream, so didn't acquire the message. Wake another
                            // receiver, and do not yet push the message.
                            msg.replace(m);
                            continue;
                        } else {
                            // Was async and not a stream, so it did acquire the message. Push the
                            // message to the queue for it to be received.
                            chan.queue.push_back(m);
                            drop(chan);
                            break;
                        }
                    }
                    Some((None, signal)) => {
                        drop(chan);
                        signal.fire();
                        break; // Was sync, so it has acquired the message
                    }
                }
            }

            Ok(()).into()
        } else if chan
            .sending
            .as_ref()
            .map(|(cap, _)| chan.queue.len() < *cap)
            .unwrap_or(true)
        {
            chan.queue.push_back(msg);
            Ok(()).into()
        } else if should_block {
            // Only bounded from here on
            let hook = make_signal(msg);
            chan.sending.as_mut().unwrap().1.push_back(hook.clone());
            drop(chan);

            do_block(hook)
        } else {
            Err(SendError(msg)).into()
        }
    }

    fn recv<S: Signal, R: From<Result<T, RecvError>>>(
        &self,
        should_block: bool,
        make_signal: impl FnOnce() -> Arc<Hook<T, S>>,
        do_block: impl FnOnce(Arc<Hook<T, S>>) -> R,
    ) -> R {
        let mut chan = wait_lock(&self.chan);
        chan.pull_pending(true);

        if let Some(msg) = chan.queue.pop_front() {
            drop(chan);
            Ok(msg).into()
        } else if self.is_disconnected() {
            drop(chan);
            Err(RecvError::Disconnected).into()
        } else if should_block {
            let hook = make_signal();
            chan.waiting.push_back(hook.clone());
            drop(chan);

            do_block(hook)
        } else {
            drop(chan);
            Err(RecvError::Disconnected).into()
        }
    }

    fn recv_sync(&self, block: Option<Option<Instant>>) -> Result<T, RecvError> {
        self.recv(
            // should_block
            block.is_some(),
            // make_signal
            || Hook::slot(None, signal::SyncSignal::default()),
            // do_block
            |hook| {
                if let Some(deadline) = block.unwrap() {
                    hook.wait_deadline_recv(&self.disconnected, deadline)
                        .or_else(|timed_out| {
                            if timed_out {
                                // Remove our signal
                                let hook: Arc<Hook<T, dyn Signal>> = hook.clone();
                                wait_lock(&self.chan)
                                    .waiting
                                    .retain(|s| s.signal().as_ptr() != hook.signal().as_ptr());
                            }
                            match hook.try_take() {
                                Some(msg) => Ok(msg),
                                None => {
                                    if let Some(msg) = wait_lock(&self.chan).queue.pop_front() {
                                        Ok(msg)
                                    } else {
                                        Err(RecvError::Disconnected)
                                    }
                                }
                            }
                        })
                } else {
                    hook.wait_recv(&self.disconnected)
                        .or_else(|| wait_lock(&self.chan).queue.pop_front())
                        .ok_or(RecvError::Disconnected)
                }
            },
        )
    }

    /// Disconnect anything listening on this channel (this will not prevent receivers receiving
    /// msgs that have already been sent)
    fn disconnect_all(&self) {
        self.disconnected.store(true, Ordering::Relaxed);

        let mut chan = wait_lock(&self.chan);
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

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        let mut chan = wait_lock(&self.chan);
        chan.pull_pending(false);
        chan.queue.len()
    }

    fn is_disconnected(&self) -> bool {
        self.disconnected.load(Ordering::SeqCst)
    }
}

mod signal {
    use spin::Mutex as Spinlock;
    use std::{
        any::Any,
        sync::atomic::{AtomicBool, Ordering},
        task::{Context, Waker},
        thread::{self, Thread},
        time::Duration,
    };

    use super::Hook;

    pub trait Signal: Send + Sync + 'static {
        /// Fire the signal, returning whether it is a stream signal. This is because streams do not
        /// acquire a message when woken, so signals must be fired until one that does acquire a message
        /// is fired, otherwise a wakeup could be missed, leading to a lost message until one is eagerly
        /// grabbed by a receiver.
        fn fire(&self) -> bool;
        fn as_any(&self) -> &(dyn Any + 'static);
        fn as_ptr(&self) -> *const ();
    }

    pub struct SyncSignal(Thread);

    impl Default for SyncSignal {
        fn default() -> Self {
            Self(thread::current())
        }
    }

    impl Signal for SyncSignal {
        fn fire(&self) -> bool {
            self.0.unpark();
            false
        }
        fn as_any(&self) -> &(dyn Any + 'static) {
            self
        }
        fn as_ptr(&self) -> *const () {
            self as *const _ as *const ()
        }
    }

    impl SyncSignal {
        pub fn wait(&self) {
            thread::park();
        }

        pub fn wait_timeout(&self, dur: Duration) {
            thread::park_timeout(dur);
        }
    }

    pub struct AsyncSignal {
        waker: Spinlock<Waker>,
        pub(super) woken: AtomicBool,
        stream: bool,
    }

    impl AsyncSignal {
        pub fn new(cx: &Context, stream: bool) -> Self {
            AsyncSignal {
                waker: Spinlock::new(cx.waker().clone()),
                woken: AtomicBool::new(false),
                stream,
            }
        }
    }

    impl Signal for AsyncSignal {
        fn fire(&self) -> bool {
            self.woken.store(true, Ordering::SeqCst);
            self.waker.lock().wake_by_ref();
            self.stream
        }

        fn as_any(&self) -> &(dyn Any + 'static) {
            self
        }
        fn as_ptr(&self) -> *const () {
            self as *const _ as *const ()
        }
    }

    impl<T> Hook<T, AsyncSignal> {
        pub fn update_waker(&self, cx_waker: &Waker) {
            if !self.1.waker.lock().will_wake(cx_waker) {
                *self.1.waker.lock() = cx_waker.clone();

                // Avoid the edge case where the waker was woken just before the wakers were
                // swapped.
                if self.1.woken.load(Ordering::SeqCst) {
                    cx_waker.wake_by_ref();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{executor::block_on, prelude::*, stream::FuturesUnordered};
    use std::{
        future::Future,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        task::{Context, Poll, Waker},
        time::Duration,
    };

    #[test]
    fn async_recv() {
        let (tx, rx) = unbounded();

        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            block_on(tx.send(42u32)).unwrap();
        });

        block_on(async {
            assert_eq!(rx.recv().await.unwrap(), 42);
        });

        t.join().unwrap();
    }

    #[test]
    fn async_send() {
        let (tx, rx) = bounded(1);

        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            assert_eq!(block_on(rx.recv()), Ok(42));
        });

        block_on(async {
            tx.send(42u32).await.unwrap();
        });

        t.join().unwrap();
    }

    #[test]
    fn async_recv_disconnect() {
        let (tx, rx) = bounded::<i32>(0);

        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            drop(tx)
        });

        block_on(async {
            assert_eq!(rx.recv().await, Err(RecvError::Disconnected));
        });

        t.join().unwrap();
    }

    #[test]
    fn async_send_disconnect() {
        let (tx, rx) = bounded(0);

        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            drop(rx)
        });

        block_on(async {
            assert_eq!(tx.send(42u32).await, Err(SendError(42)));
        });

        t.join().unwrap();
    }

    #[tokio::test]
    async fn async_recv_drop_recv() {
        let (tx, rx) = bounded::<i32>(10);

        let recv_fut = rx.recv();

        let res = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(res.is_err());

        let rx2 = rx.clone();
        let t = tokio::spawn(async move { rx2.recv().await });

        std::thread::sleep(std::time::Duration::from_millis(500));

        tx.send(42).await.unwrap();

        drop(recv_fut);

        assert_eq!(t.await.unwrap(), Ok(42))
    }

    #[tokio::test]
    async fn async_send_1_million_no_drop_or_reorder() {
        #[derive(Debug)]
        enum Message {
            Increment { old: u64 },
            ReturnCount,
        }

        let (tx, rx) = unbounded();

        let t = tokio::spawn(async move {
            let mut count = 0u64;

            while let Ok(Message::Increment { old }) = rx.recv().await {
                assert_eq!(old, count);
                count += 1;
            }

            count
        });

        for next in 0..1_000_000 {
            tx.send(Message::Increment { old: next }).await.unwrap();
        }

        tx.send(Message::ReturnCount).await.unwrap();

        let count = t.await.unwrap();
        assert_eq!(count, 1_000_000)
    }

    #[tokio::test]
    async fn parallel_async_receivers() {
        let (tx, rx) = unbounded();
        let send_fut = async move {
            let n_sends: usize = 100000;
            for _ in 0..n_sends {
                tx.send(()).await.unwrap();
            }
        };

        tokio::spawn(
            tokio::time::timeout(Duration::from_secs(5), send_fut)
                .map_err(|_| panic!("Send timed out!")),
        );

        let mut futures_unordered = (0..250)
            .map(|_| async {
                while let Ok(()) = rx.recv().await
                /* rx.recv() is OK */
                {}
            })
            .collect::<FuturesUnordered<_>>();

        let recv_fut = async { while futures_unordered.next().await.is_some() {} };

        tokio::time::timeout(Duration::from_secs(5), recv_fut)
            .map_err(|_| panic!("Receive timed out!"))
            .await
            .unwrap();

        println!("recv end");
    }

    #[test]
    fn change_waker() {
        let (tx, rx) = bounded(1);
        block_on(tx.send(())).unwrap();

        struct DebugWaker(Arc<AtomicUsize>, Waker);

        impl DebugWaker {
            fn new() -> Self {
                let woken = Arc::new(AtomicUsize::new(0));
                let woken_cloned = woken.clone();
                let waker = waker_fn::waker_fn(move || {
                    woken.fetch_add(1, Ordering::SeqCst);
                });
                DebugWaker(woken_cloned, waker)
            }

            fn woken(&self) -> usize {
                self.0.load(Ordering::SeqCst)
            }

            fn ctx(&self) -> Context {
                Context::from_waker(&self.1)
            }
        }

        // Check that the waker is correctly updated when sending tasks change their wakers
        {
            let send_fut = tx.send(());
            futures::pin_mut!(send_fut);

            let (waker1, waker2) = (DebugWaker::new(), DebugWaker::new());

            // Set the waker to waker1
            assert_eq!(send_fut.as_mut().poll(&mut waker1.ctx()), Poll::Pending);

            // Change the waker to waker2
            assert_eq!(send_fut.poll(&mut waker2.ctx()), Poll::Pending);

            // Wake the future
            block_on(rx.recv()).unwrap();

            // Check that waker2 was woken and waker1 was not
            assert_eq!(waker1.woken(), 0);
            assert_eq!(waker2.woken(), 1);
        }

        // Check that the waker is correctly updated when receiving tasks change their wakers
        {
            block_on(rx.recv()).unwrap();
            let recv_fut = rx.recv();
            futures::pin_mut!(recv_fut);

            let (waker1, waker2) = (DebugWaker::new(), DebugWaker::new());

            // Set the waker to waker1
            assert_eq!(recv_fut.as_mut().poll(&mut waker1.ctx()), Poll::Pending);

            // Change the waker to waker2
            assert_eq!(recv_fut.poll(&mut waker2.ctx()), Poll::Pending);

            // Wake the future
            block_on(tx.send(())).unwrap();

            // Check that waker2 was woken and waker1 was not
            assert_eq!(waker1.woken(), 0);
            assert_eq!(waker2.woken(), 1);
        }
    }

    #[test]
    fn spsc_single_threaded_value_ordering() {
        async fn test() {
            let (tx, rx) = bounded(4);
            tokio::select! {
                _ = producer(tx) => {},
                _ = consumer(rx) => {},
            }
        }

        async fn producer(tx: Sender<usize>) {
            for i in 0..100 {
                tx.send(i).await.unwrap();
            }
        }

        async fn consumer(rx: Receiver<usize>) {
            let mut expected = 0;
            while let Ok(value) = rx.recv().await {
                assert_eq!(value, expected);
                expected += 1;
            }
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(test());
    }

    #[test]
    fn create_sender_from_receiver() {
        let (tx, rx) = unbounded();
        let tx2 = rx.create_sender();
        drop(tx);

        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            assert_eq!(block_on(rx.recv()), Ok(43));
        });

        block_on(async {
            tx2.send(43u32).await.unwrap();
        });

        t.join().unwrap();
    }
}
