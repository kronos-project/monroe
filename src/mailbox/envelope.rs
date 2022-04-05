use std::marker::PhantomData;

use futures_core::future::LocalBoxFuture;

use super::OneshotSender;
use crate::{Actor, Context, Handler, Message};

/// A crate-internal helper trait for erasing different envelope
/// types in an actor's mailbox.
///
/// Heap-allocated envelope proxies will be sent to mailboxes and
/// are free to implement custom strategies for message handling
/// in [`EnvelopeProxy::deliver`] without the actor needing to know
/// the internals.
pub trait EnvelopeProxy {
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
    ) -> LocalBoxFuture<'a, ()>;
}

/// A fire-and-forget envelope for implementing the *tell* strategy.
pub struct ForgettingEnvelope<A, M> {
    message: M,
    _a: PhantomData<fn() -> A>,
}

impl<A, M: Message> ForgettingEnvelope<A, M> {
    pub fn new(message: M) -> Self {
        Self {
            message,
            _a: PhantomData,
        }
    }
}

impl<A: Actor + Handler<M>, M: Message> EnvelopeProxy for ForgettingEnvelope<A, M> {
    type Actor = A;

    fn deliver<'a>(
        self: Box<Self>,
        actor: &'a mut Self::Actor,
        ctx: &'a mut Context<Self::Actor>,
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let Self { message, .. } = *self;

            // The caller doesn't care about the handler's
            // return type here, so we will just discard it.
            let _ = <Self::Actor as Handler<M>>::handle(actor, message, ctx).await;
        })
    }
}

/// An envelope that returns the response to a message through
/// a oneshot channel.
pub struct ReturningEnvelope<A, M: Message> {
    message: M,
    tx: OneshotSender<M::Result>,
    _a: PhantomData<fn() -> A>,
}

impl<A, M: Message> ReturningEnvelope<A, M> {
    pub fn new(message: M, tx: OneshotSender<M::Result>) -> Self {
        Self {
            message,
            tx,
            _a: PhantomData,
        }
    }
}

impl<A: Actor + Handler<M>, M: Message> EnvelopeProxy for ReturningEnvelope<A, M> {
    type Actor = A;

    fn deliver<'a>(
        self: Box<Self>,
        actor: &'a mut Self::Actor,
        ctx: &'a mut Context<Self::Actor>,
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let Self { message, tx, .. } = *self;

            let result = <Self::Actor as Handler<M>>::handle(actor, message, ctx).await;
            let _ = tx.send(result);
        })
    }
}

// TODO: `RequestingEnvelope` implementation.
