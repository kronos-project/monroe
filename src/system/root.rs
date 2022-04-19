use std::{any::Any, fmt, marker::PhantomData, mem};

use futures_util::future::{ready, Ready};
use rustc_hash::FxHashMap;
use tokio::task::JoinHandle;

use super::ActorHandle;
use crate::{Actor, Address, Context, Handler, Message, ROOT_ACTOR_ID};

#[derive(Debug)]
pub struct RootActor {
    actor_addresses: FxHashMap<u64, Box<dyn Any + Send + 'static>>,
    actor_handles: Vec<JoinHandle<()>>,
}

impl RootActor {
    pub fn new(ctx: &mut Context<Self>) -> Self {
        debug_assert_eq!(ctx.id(), ROOT_ACTOR_ID);

        // TODO: What is a reasonable default capacity?
        Self {
            actor_addresses: FxHashMap::default(),
            actor_handles: Vec::new(),
        }
    }
}

impl Actor for RootActor {
    type StartingFuture<'a> = Ready<()>;
    type StoppedFuture = Ready<()>;

    fn starting(&mut self, _ctx: &mut Context<Self>) -> Self::StartingFuture<'_> {
        ready(())
    }

    fn stopped(&mut self) -> Self::StoppedFuture {
        ready(())
    }
}

pub struct AddActor<A: Actor>(pub ActorHandle<A>);

impl<A: Actor> Message for AddActor<A> {
    type Result = ();
}

impl<A: Actor> Handler<AddActor<A>> for RootActor {
    type HandleFuture<'a> = Ready<()>;

    fn handle(&mut self, message: AddActor<A>, _ctx: &mut Context<Self>) -> Self::HandleFuture<'_> {
        let ActorHandle { address, handle } = message.0;

        // Store the information of the actor for later usage.
        self.actor_addresses.insert(address.id(), Box::new(address));
        self.actor_handles.push(handle);

        ready(())
    }
}

#[derive(Debug)]
pub struct DrainActors;

impl Message for DrainActors {
    type Result = Vec<JoinHandle<()>>;
}

impl Handler<DrainActors> for RootActor {
    type HandleFuture<'a> = Ready<Vec<JoinHandle<()>>>;

    fn handle(&mut self, _: DrainActors, _ctx: &mut Context<Self>) -> Self::HandleFuture<'_> {
        // We take all the handles out instead of joining them here
        // so as to not prevent `RootActor` from processing anymore
        // messages. We leave `actor_addresses` as-is because our
        // goal is for actors to terminate naturally and not through
        // dropped reference counts.
        ready(mem::take(&mut self.actor_handles))
    }
}

pub struct FindActor<A: Actor> {
    id: u64,
    _a: PhantomData<fn() -> A>,
}

impl<A: Actor> FindActor<A> {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            _a: PhantomData,
        }
    }
}

impl<A: Actor> fmt::Debug for FindActor<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FindActor").field("id", &self.id).finish()
    }
}

impl<A: Actor> Message for FindActor<A> {
    type Result = Option<Address<A>>;
}

impl<A: Actor> Handler<FindActor<A>> for RootActor {
    type HandleFuture<'a> = Ready<Option<Address<A>>>;

    fn handle(
        &mut self,
        message: FindActor<A>,
        _ctx: &mut Context<Self>,
    ) -> Self::HandleFuture<'_> {
        ready(
            self.actor_addresses
                .get(&message.id)
                .and_then(|addr| addr.downcast_ref::<Address<A>>().cloned()),
        )
    }
}
