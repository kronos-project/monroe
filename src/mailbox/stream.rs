use std::marker::PhantomData;

use futures_core::future::BoxFuture;

use super::EnvelopeProxy;
use crate::{Actor, Context, StreamHandler};

enum StreamNotification<I> {
    Started,
    Item(I),
    Stopped,
}

/// An envelope that notifies the actor about state changes for
/// stream handling.
pub struct StreamEnvelope<A, I> {
    notification: StreamNotification<I>,
    _a: PhantomData<fn() -> A>,
}

impl<A, I> StreamEnvelope<A, I> {
    pub fn started() -> Self {
        Self {
            notification: StreamNotification::Started,
            _a: PhantomData,
        }
    }

    pub fn item(item: I) -> Self {
        Self {
            notification: StreamNotification::Item(item),
            _a: PhantomData,
        }
    }

    pub fn stopped() -> Self {
        Self {
            notification: StreamNotification::Stopped,
            _a: PhantomData,
        }
    }
}

impl<A: Actor + StreamHandler<I>, I: Send + 'static> EnvelopeProxy for StreamEnvelope<A, I> {
    type Actor = A;

    fn deliver<'a>(
        self: Box<Self>,
        actor: &'a mut Self::Actor,
        ctx: &'a mut Context<Self::Actor>,
    ) -> BoxFuture<'a, ()> {
        use StreamNotification::*;
        match self.notification {
            Item(item) => Box::pin(<Self::Actor as StreamHandler<I>>::handle(actor, item, ctx)),
            Started => Box::pin(<Self::Actor as StreamHandler<I>>::started(actor, ctx)),
            Stopped => Box::pin(<Self::Actor as StreamHandler<I>>::finished(actor, ctx)),
        }
    }
}
