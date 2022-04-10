use std::future::Future;

use crate::Context;

// TODO: Actor context.
// TODO: Supervision.

/// TODO
pub trait Actor: Sized + Send + 'static {
    /// The [`Future`] type produced by [`Actor::starting`].
    type StartingFuture<'a>: Future<Output = ()> + Send + 'a
    where
        Self: 'a;

    /// The [`Future`] type produced by [`Actor::stopped`].
    type StoppedFuture: Future<Output = ()> + Send;

    /// Called when an actor is in the process of starting up.
    ///
    /// During the execution of this method, no messages will
    /// be processed yet. Should be used for initialization
    /// work.
    fn starting(&mut self, ctx: &mut Context<Self>) -> Self::StartingFuture<'_>;

    /// Called when an actor has been permanently stopped.
    ///
    /// This consumes the actor object and will be responsible
    /// for dropping it. Should be used for final cleanup work.
    fn stopped(self) -> Self::StoppedFuture;
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
