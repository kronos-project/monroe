use std::marker::PhantomData;

use futures_core::future::BoxFuture;

use super::{OneshotSender, EnvelopeProxy};
use crate::{Actor, Context, Handler, Message};

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
    ) -> BoxFuture<'a, ()> {
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
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Self { message, tx, .. } = *self;

            let result = <Self::Actor as Handler<M>>::handle(actor, message, ctx).await;
            let _ = tx.send(result);
        })
    }
}

// TODO: `RequestingEnvelope` implementation.
