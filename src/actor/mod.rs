use std::future::Future;

// We want `Supervisor` in scope for more convenient doc references.
#[allow(unused_imports)]
use crate::{supervisor::Supervisor, Context};

mod new;
pub use self::new::NewActor;

mod stream;
pub use self::stream::StreamHandler;

/// An encapsulated unit of computation.
///
/// Actors are finite state machines which communicate merely
/// by exchanging [`Message`]s. They don't share their state
/// with the outer world and remove the need for synchronization
/// primitives in concurrent programs.
///
/// # Supervision
///
/// Every actor in the `monroe` crate has a [`Supervisor`]
/// associated which is consulted in different scenarios of
/// failures for the creation of self-healing systems.
///
/// An implementation of the [`NewActor`] trait is required
/// to describe the construction of an actor object for
/// supervisor-triggered restarts.
///
/// # Handling messages
///
/// Actors may have many implementations of the [`Handler`]
/// trait for messages they want to support.
///
/// # Exception safety
///
/// A big aspect of designing robust actors lies in the notion
/// of [exception safety].
///
/// Actors may panic at every stage of their lifecycle. These
/// panics are caught by us and forwarded to [`Supervisor::on_panic`]
/// (*unless panics abort instead of unwind*).
///
/// According to the supervision strategy, an actor object
/// may either be
///
/// - *restarted*; the old actor object is attempted to be
///   replaced by a new one before calling [`Actor::starting`]
///   again.
///
///   If this fails to the point of
///   [`Supervisor::on_second_restart_failure`] being called,
///   [`Actor::stopped`] will be invoked with the **unmodified**
///   actor object.
///
/// - *stopped*; in which case [`Actor::stopped`] will be called
///   directly with the actor's current state.
///
/// As a result, the implementation of [`Actor::stopped`] must
/// be crafted with the idea in mind that an actor may have just
/// been interrupted right in the middle of a [`Message`]
/// [`Handler`].
///
/// No particular logical invariants should therefore be assumed
/// on the actor object during the execution of these two methods.
/// Message handlers on the other hand can assume any logical
/// invariants are upheld at any given point unless they
/// temporarily break them on their own.
///
/// Another way of designing exception-safe actors is to
/// decouple problematic logic from the actor itself by designing
/// interfaces that are exception safe on their own, i.e. do
/// not expose their invariants to the user.
///
/// While this class of issue is generally very rare in Rust
/// and panics also shouldn't act as a generic error handling
/// mechanism, these contracts must be upheld so that we can
/// allow sane resource cleanup even on crashes.
///
/// [exception safety]: https://github.com/rust-lang/rfcs/blob/master/text/1236-stabilize-catch-panic.md
// TODO: Explain how actors are started.
pub trait Actor: Sized + Send + 'static {
    /// The [`Future`] type produced by [`Actor::starting`].
    type StartingFuture<'a>: Future<Output = ()> + Send + 'a
    where
        Self: 'a;

    /// The [`Future`] type produced by [`Actor::stopped`].
    type StoppedFuture: Future<Output = ()> + Send;

    /// Called when the actor is in the process of starting up.
    ///
    /// This is also naturally called after an actor was
    /// *restarted* by its [`Supervisor`].
    ///
    /// During the execution of this method, no messages will
    /// be processed yet. Can be used for initialization work.
    fn starting(&mut self, ctx: &mut Context<Self>) -> Self::StartingFuture<'_>;

    /// Called when the actor has been permanently stopped.
    ///
    /// As the actor is considered stopped at this point, no
    /// execution [`Context`] is provided to this method.
    ///
    /// Final cleanup work may be performed before the actor
    /// object itself will be dropped.
    ///
    /// # Exception safety
    ///
    /// As detailed in the documentation for [`Actor`], this
    /// method should not assume logical invariants on the
    /// actor object as they may have been violated under the
    /// circumstances of the shutdown.
    fn stopped(&mut self) -> Self::StoppedFuture;

    // TODO: pre/post restart methods?
}

/// Representation of a message type that can be sent to an actor.
///
/// Messages of certain types can be sent to [`Actor`]s with an
/// implementation of the corresponding [`Handler`] trait.
///
/// Generally, monroe features 3 ways of message passing out of the
/// box through [`Address`][crate::Address]:
///
/// - **tell**: A message will be sent to an actor without waiting
///   for its response. This will return immediately and can never
///   cause deadlocks.
///
/// - **ask**: A message will be sent to an actor and its [`Message::Result`]
///   response will be awaited. This can potentially cause deadlocks
///   when actors *ask* each other in a cyclic relationship.
///
/// - **request**: A message will be sent to an actor similarly to
///   the *tell* strategy. But instead of just discarding, the
///   [`Message::Result`] produced by that actor will be scheduled
///   as a distinct [`Message`] to the requesting [`Actor`] at a later
///   point in the future.
///
///   This requires that a requesting [`Actor`] is a handler of a
///   [`Message::Result`] associated with the [`Message`] it is
///   trying to send. This strategy can never deadlock.
///
/// An [`Actor`] can support and handle several and arbitrary types of
/// messages as long as it has enough implementations of the [`Handler`]
/// trait.
pub trait Message: Send + 'static {
    /// The result type that is produced when [`Handler::handle`] is
    /// called on an [`Actor`] for this message.
    ///
    /// See [`Message`] documentation for different ways of sending
    /// messages and how the result value will be passed back.
    type Result: Send + 'static;
}

/// Defines handling semantics for a [`Message`] of type `M` on
/// an [`Actor`].
///
/// This will effectively enable the actor to receive and process
/// messages of type `M`.
///
/// Multiple implementations of the [`Handler`] trait can be done
/// when an [`Actor`] is supposed to handle multiple [`Message`]
/// types.
pub trait Handler<M: Message>: Actor {
    /// The [`Future`] type produced by [`Handler::handle`].
    type HandleFuture<'a>: Future<Output = M::Result> + Send + 'a
    where
        Self: 'a,
        M: 'a;

    /// Processes a message of type `M` when the actor receives it.
    ///
    /// Only one message is ever processed at a time, so the inner
    /// state of the [`Actor`] can be safely mutated here without
    /// the need for synchronization.
    ///
    /// The handler must produce a [`Message::Result`] value which
    /// will eventually be passed back to the issuer of the message.
    // TODO: Write about the context's role.
    fn handle(&mut self, message: M, ctx: &mut Context<Self>) -> Self::HandleFuture<'_>;
}
