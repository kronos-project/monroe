use std::{fmt, time::Duration};

use monroe_inbox::{SendError, SendTimeoutError, TrySendError};
use tokio::sync::oneshot;

use crate::{
    mailbox::{ForgettingEnvelope, Letter, MailboxSender, ReturningEnvelope, WeakMailboxSender},
    Actor, Handler, Message,
};

/// The message could not be delivered to the actor because
/// it was already shut down.
pub type Disconnected<T> = SendError<T>;

/// The error that is produced by [`Address::try_tell`].
pub type TellError<T> = TrySendError<T>;

/// The error that is produced by [`Address::tell_timeout`].
pub type TellTimeoutError<T> = SendTimeoutError<T>;

/// The error that is produced by [`Address::try_ask`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TryAskError<T> {
    /// The envelope has been dropped before it could produce
    /// an answer.
    ///
    /// This is usually the case when an actor panics while
    /// processing a message or when the actor task is aborted
    /// by force.
    Dropped,
    /// The channel the message is sent on has a finite capacity
    /// and was full when the send was attempted.
    Full(T),
    /// The message could not be delivered to the actor because
    /// it was already shut down.
    Disconnected(T),
}

/// The error that is produced by [`Address::ask`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskError<T> {
    /// The envelope has been dropped before it could produce
    /// an answer.
    ///
    /// This is usually the case when an actor panics while
    /// processing a message or when the actor task is aborted
    /// by force.
    Dropped,
    /// The message could not be delivered to the actor because
    /// it was already shut down.
    Disconnected(T),
}

/// The error that is produced by [`Address::ask_timeout`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskTimeoutError<T> {
    /// The envelope has been dropped before it could produce
    /// an answer.
    ///
    /// This is usually the case when an actor panics while
    /// processing a message or when the actor task is aborted
    /// by force.
    Dropped,
    /// A timeout occurred when attempting to send the message.
    Timeout(T),
    /// All channel receivers were dropped and so the message
    /// has nobody to receive it.
    Disconnected(T),
}

// TODO: Error trait impls for all these types.

/// A strong reference to an [`Actor`].
///
/// [`Address`]es serve as handles to isolated actor processes
/// and allow for communication with them.
///
/// Values of this type are strongly reference-counted. Within
/// an application, actors will remain active until the last
/// [`Address`] referencing them is dropped. When more addresses
/// need to be shared with other parts of the application, such
/// can be cloned cheaply.
///
/// In situations where strong reference counting semantics to
/// extend an actor's lifetime are undesired, [`WeakAddress`]es
/// can be used instead.
///
/// Please note that the existence of an [`Address`] *DOES NOT*
/// automatically guarantee that the referenced actor `A` is
/// running -- it may still crash or terminate due to its own
/// circumstances, which makes the methods for communicating
/// with the actor fallible.
///
/// When an [`Actor`] handles a message, [`Context::address`]
/// allows an actor to retrieve its own reference.
///
/// [`Context::address`]: crate::Context::address
pub struct Address<A: Actor> {
    id: u64,
    tx: MailboxSender<A>,
}

/// A weak reference to an [`Actor`].
///
/// This is the counterpart to [`Address`] and maintains a weak
/// reference count instead of a strong one.
///
/// As such, it does not actively influence the lifecycle of the
/// [`Actor`] `A` it references. This makes weak addresses fit
/// for when a reference to an actor must be kept without
/// employing the semantics of strong reference counting detailed
/// in the documentation for [`Address`].
///
/// A weak address must always assume it outlives the referenced
/// actor and thus cannot be used for communication directly. It
/// must first be made into a strong [`Address`] through the
/// [`WeakAddress::upgrade`] method which only succeeds when the
/// actor is still alive.
pub struct WeakAddress<A: Actor> {
    id: u64,
    tx: WeakMailboxSender<A>,
}

impl<A: Actor> Address<A> {
    #[inline]
    pub(crate) fn new(id: u64, tx: MailboxSender<A>) -> Self {
        Self { id, tx }
    }

    /// Gets the unique, numeric identifier associated with the
    /// actor referenced by this address.
    ///
    /// The only assumption that is safe to make about an actor's
    /// ID is that no two actors will ever share the same value.
    #[inline]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Downgrades this strong address into a [`WeakAddress`].
    ///
    /// See the documentation for [`WeakAddress`] to learn its
    /// semantics and when creating one may be desired.
    #[inline]
    pub fn downgrade(&self) -> WeakAddress<A> {
        WeakAddress {
            id: self.id,
            tx: self.tx.downgrade(),
        }
    }

    /// Whether this address is disconnected from the [`Actor`] it
    /// references.
    ///
    /// Practically speaking, this method returns `true` when the
    /// referenced actor has died and is unable to receive any more
    /// messages.
    #[inline]
    pub fn is_disconnected(&self) -> bool {
        self.tx.is_disconnected()
    }

    /// Sends a fire-and-forget [`Message`] to the actor and
    /// returns immediately.
    ///
    /// This does not await an actor's response and will fail
    /// if the actor is disconnected or its mailbox is full.
    /// In such cases, the [`TellError`] object transfers
    /// ownership over the sent message back to the caller.
    ///
    /// This method can be used with no regrets - this strategy
    /// of sending messages can never cause a deadlock.
    pub fn try_tell<M>(&self, message: M) -> Result<(), TellError<M>>
    where
        M: Message,
        A: Handler<M>,
    {
        let envelope = ForgettingEnvelope::<A, M>::new(message);

        // SAFETY: We always get `ForgettingEnvelope<A, M>` back on error.
        self.tx.try_send(Box::new(envelope)).map_err(|e| match e {
            TellError::Full(letter) => {
                let letter: Box<ForgettingEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                TellError::Full(letter.message)
            }
            TellError::Disconnected(letter) => {
                let letter: Box<ForgettingEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                TellError::Disconnected(letter.message)
            }
        })
    }

    /// Sends a fire-and-forget [`Message`] to the actor.
    ///
    /// Unlike [`Address::try_tell`], this method is asynchronous
    /// and will wait for a free slot in the actor mailbox
    /// prior to returning.
    ///
    /// This also means that actors with bounded mailboxes
    /// must **beware of deadlocks on cyclic interaction**.
    /// For an operation that prevents deadlocks, have a look
    /// at the [`try_tell`](Address::try_tell) method.
    ///
    /// [`Disconnected`] will be returned when the referenced
    /// actor has shut down and does not accept messages.
    pub async fn tell<M>(&self, message: M) -> Result<(), Disconnected<M>>
    where
        M: Message,
        A: Handler<M>,
    {
        let envelope = ForgettingEnvelope::<A, M>::new(message);

        // SAFETY: We always get `ForgettingEnvelope<A, M>` back on error.
        self.tx
            .send(Box::new(envelope))
            .await
            .map_err(|SendError(letter)| {
                let letter: Box<ForgettingEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                SendError(letter.message)
            })
    }

    /// Sends a fire-and-forget [`Message`] to the actor.
    ///
    /// Unlike [`Address::tell`], this method is asynchronous
    /// and will either wait for a free slot in the actor mailbox
    /// prior to returning, or until the given timeout expired.
    ///
    /// This method can be used as a way to prevent deadlocks
    /// when sending messages since it will stop waiting when
    /// the timeout has expired.
    ///
    /// [`Disconnected`] will be returned when the referenced
    /// actor has shut down and does not accept messages.
    pub async fn tell_timeout<M>(
        &self,
        message: M,
        timeout: Duration,
    ) -> Result<(), TellTimeoutError<M>>
    where
        M: Message,
        A: Handler<M>,
    {
        let envelope = ForgettingEnvelope::<A, M>::new(message);

        // SAFETY: We always get `ForgettingEnvelope<A, M>` back on error.
        self.tx
            .send_timeout(Box::new(envelope), timeout)
            .await
            .map_err(|error| match error {
                TellTimeoutError::Timeout(letter) => {
                    let letter: Box<ForgettingEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                    TellTimeoutError::Timeout(letter.message)
                }
                TellTimeoutError::Disconnected(letter) => {
                    let letter: Box<ForgettingEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                    TellTimeoutError::Disconnected(letter.message)
                }
            })
    }

    ///
    pub async fn try_ask<M>(&self, message: M) -> Result<M::Result, TryAskError<M>>
    where
        M: Message,
        A: Handler<M>,
    {
        let (tx, rx) = oneshot::channel();
        let envelope = ReturningEnvelope::<A, M>::new(message, tx);

        // SAFETY: We always get `ReturningEnvelope<A, M>` back on error.
        match self.tx.try_send(Box::new(envelope)) {
            Ok(()) => match rx.await {
                Ok(res) => Ok(res),
                Err(_) => Err(TryAskError::Dropped),
            },
            Err(TrySendError::Full(letter)) => {
                let letter: Box<ReturningEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                Err(TryAskError::Full(letter.message))
            }
            Err(TrySendError::Disconnected(letter)) => {
                let letter: Box<ReturningEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                Err(TryAskError::Disconnected(letter.message))
            }
        }
    }

    ///
    pub async fn ask<M>(&self, message: M) -> Result<M::Result, AskError<M>>
    where
        A: Actor + Handler<M>,
        M: Message,
    {
        let (tx, rx) = oneshot::channel();
        let envelope = ReturningEnvelope::<A, M>::new(message, tx);

        match self.tx.send(Box::new(envelope)).await {
            Ok(()) => match rx.await {
                Ok(res) => Ok(res),
                Err(_) => Err(AskError::Dropped),
            },
            Err(SendError(letter)) => {
                let letter: Box<ReturningEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                Err(AskError::Disconnected(letter.message))
            }
        }
    }

    ///
    pub async fn ask_timeout<M>(
        &self,
        message: M,
        timeout: Duration,
    ) -> Result<M::Result, AskTimeoutError<M>>
    where
        A: Actor + Handler<M>,
        M: Message,
    {
        let (tx, rx) = oneshot::channel();
        let envelope = ReturningEnvelope::<A, M>::new(message, tx);

        match self.tx.send_timeout(Box::new(envelope), timeout).await {
            Ok(()) => match rx.await {
                Ok(res) => Ok(res),
                Err(_) => Err(AskTimeoutError::Dropped),
            },
            Err(SendTimeoutError::Timeout(letter)) => {
                let letter: Box<ReturningEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                Err(AskTimeoutError::Timeout(letter.message))
            }
            Err(SendTimeoutError::Disconnected(letter)) => {
                let letter: Box<ReturningEnvelope<A, M>> = unsafe { downcast_letter(letter) };
                Err(AskTimeoutError::Disconnected(letter.message))
            }
        }
    }
}

impl<A: Actor> WeakAddress<A> {
    /// Gets the unique, numeric identifier associated with the
    /// actor referenced by this address.
    ///
    /// The only assumption that is safe to make about an actor's
    /// ID is that no two actors will ever share the same value.
    #[inline]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Attempts to upgrade this weak address to an [`Address`].
    ///
    /// On success, the produced [`Address`] value will have the
    /// typical strong reference counting semantics and will
    /// therefore actively influence actor `A`'s lifetime.
    ///
    /// This method will return [`None`] when actor `A` has already
    /// been dropped.
    #[inline]
    pub fn upgrade(&self) -> Option<Address<A>> {
        self.tx.upgrade().map(|tx| Address { id: self.id, tx })
    }
}

impl<A: Actor> Clone for Address<A> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            tx: self.tx.clone(),
        }
    }
}

impl<A: Actor> Clone for WeakAddress<A> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            tx: self.tx.clone(),
        }
    }
}

impl<A: Actor> fmt::Debug for Address<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Address").field("id", &self.id).finish()
    }
}

impl<A: Actor> fmt::Debug for WeakAddress<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakAddress").field("id", &self.id).finish()
    }
}

impl<A: Actor> PartialEq for Address<A> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Addresses are automatically equal when the
        // unique IDs of the referenced actors match.
        self.id == other.id
    }
}

impl<A: Actor> PartialEq for WeakAddress<A> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Addresses are automatically equal when the
        // unique IDs of the referenced actors match.
        self.id == other.id
    }
}

// TODO: Should (Weak)Address implement Hash?

#[inline(always)]
unsafe fn downcast_letter<A, T>(letter: Letter<A>) -> Box<T> {
    let ptr = Box::into_raw(letter);
    unsafe { Box::from_raw(ptr as *mut T) }
}
