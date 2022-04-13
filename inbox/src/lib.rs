//! Implementation of a custom mpsc channel that is based on
//! [`flume`].
//!
//! This is mostly a stripped version of [`flume`] with some
//! additional features that we need for monroe.
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
use parking_lot::{Mutex, MutexGuard};
use signal::Signal;
use std::{
    collections::VecDeque,
    num::NonZeroUsize,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

/// Create a channel with no maximum capacity.
///
/// Create an unbounded channel with a [`Sender`] and
/// [`Receiver`] connected to each end respectively. Values sent
/// in one end of the channel will be received on the other end.
/// The channel is thread-safe, and both [`Sender`] and
/// [`Receiver`] may be sent to or shared between threads as
/// necessary. In addition, both [`Sender`] and [`Receiver`] may
/// be cloned.
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
/// Create a bounded channel with a [`Sender`] and [`Receiver`]
/// connected to each end respectively. Values sent in one end of
/// the channel will be received on the other end. The channel is
/// thread-safe, and both [`Sender`] and [`Receiver`] may be sent
/// to or shared between threads as necessary.
///
/// Unlike an [`unbounded`] channel, if there is no space left
/// for new messages, calls to [`Sender::send`] will block
/// (unblocking once a receiver has made space). If blocking
/// behaviour is not desired, [`Sender::try_send`] may be used.
///
/// Unlike `std::sync::mpsc`, this channel does not support
/// channels with a capacity of 0.  Calling this method with `0`
/// as the cap will lead to a panic.
pub fn bounded<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    let cap = NonZeroUsize::new(cap).expect("Tried to create a channel with 0 capacity");
    let shared = Arc::new(Shared::new(Some(cap)));
    (
        Sender {
            shared: shared.clone(),
        },
        Receiver { shared },
    )
}

pub(crate) type SignalVec<T> = VecDeque<Arc<T>>;

pub(crate) struct Chan<T> {
    // `sending` is true if this is a bounded channel and the
    // first element of the tuple is the cap. The latter element
    // is a List of senders that are waiting for the channel to
    // becompe empty
    sending: Option<(NonZeroUsize, SignalVec<SenderHook<T, dyn Signal>>)>,
    // queue contains all items that are waiting for a receiver
    // to pick them up.
    queue: VecDeque<T>,
    // `waiting` contains all current receivers.  The `Hook`
    // inside is responsible for telling the receiver that a
    // message is ready to be taken out of the `queue`
    waiting: SignalVec<ReceiverHook<T, dyn Signal>>,
}

impl<T> Chan<T> {
    /// This method will go through every waiting sender in the
    /// `waiting` list, take their message, fire their signal,
    /// and push the message into `queue`, until the capacity of
    /// this channel has reached.
    ///
    /// If `pull_extra` is `true`, the capacity will be increased
    /// by one.
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
    fn new(cap: Option<NonZeroUsize>) -> Self {
        Self {
            chan: Mutex::new(Chan {
                sending: cap.map(|cap| (cap, VecDeque::new())),
                queue: VecDeque::new(),
                waiting: VecDeque::new(),
            }),
            disconnected: AtomicBool::new(false),
            sender_count: AtomicUsize::new(1),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        let mut chan = self.chan.lock();
        chan.pull_pending(false);
        chan.queue.len()
    }

    fn is_disconnected(&self) -> bool {
        self.disconnected.load(Ordering::SeqCst)
    }

    /// Disconnect anything listening on this channel (this will
    /// not prevent receivers receiving msgs that have already
    /// been sent)
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

    /// Returns `Ok(None)` if there's currently no message
    /// available and no do_block closure was supplied.
    pub fn recv<E: From<RecvError>, R: From<Result<T, E>>>(
        &self,
        do_block: impl FnOnce(MutexGuard<'_, Chan<T>>) -> R,
    ) -> R {
        let mut chan = self.chan.lock();
        // first, fill queue with available messages from waiting senders
        chan.pull_pending(true);

        // if there's a message from the queue, return it
        if let Some(msg) = chan.queue.pop_front() {
            return R::from(Ok(msg));
        }

        // check if channel is disconnected
        if self.is_disconnected() {
            return R::from(Err(E::from(RecvError::Disconnected)));
        }

        // execute the supplied do_block function
        do_block(chan)
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.recv(|chan| {
            drop(chan);
            Err(TryRecvError::Empty)
        })
    }

    pub fn send<R: From<Result<(), SendError<T>>>>(
        &self,
        msg: T,
        do_block: impl FnOnce(T, MutexGuard<'_, Chan<T>>) -> R,
    ) -> R {
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

            receiver.signal().fire();
            chan.queue.push_back(msg);
            drop(chan);

            return R::from(Ok(()));
        }

        // if there are no waiting receivers, we will insert the
        // message into the queue, as long as the capacity is not
        // reached
        if chan
            .sending
            .as_ref()
            .map_or(true, |(cap, _)| chan.queue.len() < cap.get())
        {
            chan.queue.push_back(msg);
            return R::from(Ok(()));
        }

        // need to wait
        do_block(msg, chan)
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
    #[should_panic]
    fn try_create_channel_with_zero_cap() {
        let (_, _) = bounded::<()>(0);
    }

    #[tokio::test]
    async fn async_recv_drop_recv() {
        let (tx, rx) = bounded::<i32>(10);
        let rx = Arc::new(rx);

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

    #[tokio::test]
    async fn downgrade_and_upgrade_sender() {
        let (tx, rx) = unbounded();

        tx.send(1).await.unwrap();

        let wtx = tx.downgrade();
        tx.send(2).await.unwrap();
        wtx.upgrade().unwrap().send(3).await.unwrap();

        assert_eq!(rx.recv().await, Ok(1));
        assert_eq!(rx.recv().await, Ok(2));
        assert_eq!(rx.recv().await, Ok(3));
    }

    #[test]
    fn weak_sender_does_not_stop_dropping() {
        let (tx, rx) = unbounded::<()>();

        let wtx = tx.downgrade();
        drop(tx);

        assert!(rx.is_disconnected());
        drop(rx);

        assert!(wtx.upgrade().is_none());
    }

    #[tokio::test]
    async fn receive_messages_after_disconnect() {
        let (tx, rx) = unbounded();

        for i in 0..100 {
            tx.send(i).await.unwrap();
        }

        drop(tx);
        assert!(rx.is_disconnected());

        for i in 0..100 {
            assert_eq!(rx.recv().await, Ok(i));
        }
    }
}
