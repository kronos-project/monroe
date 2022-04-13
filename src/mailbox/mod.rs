use futures_core::future::BoxFuture;
pub use monroe_inbox::{bounded, unbounded};

use crate::{Actor, Context};

mod messages;
pub use self::messages::*;

/// A crate-internal helper trait for erasing different envelope
/// types in an actor's mailbox.
///
/// Heap-allocated envelope proxies will be sent to mailboxes and
/// are free to implement custom strategies for message handling
/// in [`EnvelopeProxy::deliver`] without the actor needing to know
/// the internals.
pub trait EnvelopeProxy: Send {
    /// The actor type the envelope is directed at.
    type Actor: Actor;

    /// Delivers the message in the envelope for processing.
    ///
    /// The internal implementation is responsible for handling
    /// result value passing and other mechanics.
    fn deliver<'a>(
        self: Box<Self>,
        actor: &'a mut Self::Actor,
        ctx: &'a mut Context<Self::Actor>,
    ) -> BoxFuture<'a, ()>;
}

// TODO: The end goal is not having to allocate every `EnvelopeProxy::deliver`
//       future on the heap. This requires us to assign a GAT to the trait
//       which currently breaks object-safety: https://github.com/rust-lang/rust/issues/81823
//
//       Once we can, switch to a pin-projected future type that unifies
//       `ForgettingEnvelope`, `ReturningEnvelope` and `RequestingEnvelope`
//       into a common return type.
//
//       We can then use something along the lines of
//       `Box<dyn for<'a> EnvelopeProxy<Actor = A, Future<'a> = ProxyFuture<...>>>`
//       in our mailboxes for minimal memory footprint.
pub type Letter<A> = Box<dyn EnvelopeProxy<Actor = A>>;

pub type MailboxSender<A> = monroe_inbox::Sender<Letter<A>>;
pub type WeakMailboxSender<A> = monroe_inbox::WeakSender<Letter<A>>;
pub type MailboxReceiver<A> = monroe_inbox::Receiver<Letter<A>>;

pub type OneshotSender<T> = tokio::sync::oneshot::Sender<T>;
pub type OneshotReceiver<T> = tokio::sync::oneshot::Receiver<T>;
