use crate::{
    mailbox::{MailboxSender, WeakMailboxSender},
    Actor,
};

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
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
pub struct WeakAddress<A: Actor> {
    id: u64,
    tx: WeakMailboxSender<A>,
}

impl<A: Actor> Address<A> {
    pub(crate) fn new(id: u64, tx: MailboxSender<A>) -> Self {
        Self { id, tx }
    }

    /// Gets the unique, numeric identifier associated with the
    /// actor referenced by this address.
    ///
    /// The only assumption that is safe to make about an actor's
    /// ID is that no two actors will ever share the same value.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Downgrades this strong address into a [`WeakAddress`].
    ///
    /// See the documentation for [`WeakAddress`] to learn its
    /// semantics and when creating one may be desired.
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
    pub fn is_disconnected(&self) -> bool {
        self.tx.is_disconnected()
    }
}

impl<A: Actor> WeakAddress<A> {
    /// Gets the unique, numeric identifier associated with the
    /// actor referenced by this address.
    ///
    /// The only assumption that is safe to make about an actor's
    /// ID is that no two actors will ever share the same value.
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
    pub fn upgrade(&self) -> Option<Address<A>> {
        self.tx.upgrade().map(|tx| Address { id: self.id, tx })
    }
}

impl<A: Actor> PartialEq for Address<A> {
    fn eq(&self, other: &Self) -> bool {
        // Addresses are automatically equal when the
        // unique IDs of the referenced actors match.
        self.id == other.id
    }
}

impl<A: Actor> PartialEq for WeakAddress<A> {
    fn eq(&self, other: &Self) -> bool {
        // Addresses are automatically equal when the
        // unique IDs of the referenced actors match.
        self.id == other.id
    }
}

// TODO: Should (Weak)Address implement Hash?
