use std::{
    any::{Any, TypeId},
    collections::hash_map::Entry,
    fmt,
    marker::PhantomData,
    mem,
};

use futures_util::future::{ready, Ready};
use rustc_hash::FxHashMap;
use tokio::task::JoinHandle;

use super::{broker::Broker, ActorHandle};
use crate::{Actor, Address, Context, Handler, Message, ROOT_ACTOR_ID, WeakAddress};

#[derive(Debug)]
pub struct RootActor {
    actor_addresses: FxHashMap<u64, Box<dyn Any + Send + 'static>>,
    actor_handles: Vec<JoinHandle<()>>,
    brokers: FxHashMap<TypeId, Box<dyn Any + Send + 'static>>,
}

impl RootActor {
    pub fn new(ctx: &mut Context<Self>) -> Self {
        debug_assert_eq!(ctx.id(), ROOT_ACTOR_ID);

        // TODO: What is a reasonable default capacity?
        Self {
            actor_addresses: FxHashMap::default(),
            actor_handles: Vec::new(),
            brokers: FxHashMap::default(),
        }
    }

    fn get_or_create_broker<M>(&mut self) -> &mut Broker<M>
    where
        M: Clone + Message<Result = ()>,
    {
        // SAFETY: We access `self.brokers` only through this method
        // which takes care of inserting correct TypeId and broker value
        // pairs. We can therefore safely unwrap downcasted broker types.
        unsafe {
            match self.brokers.entry(TypeId::of::<M>()) {
                Entry::Occupied(entry) => entry.into_mut().downcast_mut().unwrap_unchecked(),
                Entry::Vacant(entry) => entry
                    .insert(Box::<Broker<M>>::default())
                    .downcast_mut()
                    .unwrap_unchecked(),
            }
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

// --- ActorSystem messages ---

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

// --- Broker messages. ---

pub struct Subscribe<A, M>
where
    A: Actor + Handler<M>,
    M: Clone + Message<Result = ()>,
{
    address: WeakAddress<A>,
    _m: PhantomData<fn() -> M>,
}

impl<A, M> Subscribe<A, M>
where
    A: Actor + Handler<M>,
    M: Clone + Message<Result = ()>,
{
    pub fn new(address: WeakAddress<A>) -> Self {
        Self {
            address,
            _m: PhantomData,
        }
    }
}

impl<A, M> Message for Subscribe<A, M>
where
    A: Actor + Handler<M>,
    M: Clone + Message<Result = ()>,
{
    type Result = ();
}

impl<A, M> Handler<Subscribe<A, M>> for RootActor
where
    A: Actor + Handler<M>,
    M: Clone + Message<Result = ()>,
{
    type HandleFuture<'a> = Ready<()>;

    fn handle(
        &mut self,
        message: Subscribe<A, M>,
        _ctx: &mut Context<Self>,
    ) -> Self::HandleFuture<'_> {
        let broker = self.get_or_create_broker::<M>();
        broker.subscribe(message.address);

        ready(())
    }
}

pub struct Unsubscribe<M> {
    id: u64,
    _m: PhantomData<M>,
}

impl<M> Unsubscribe<M> {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            _m: PhantomData,
        }
    }
}

impl<M> Message for Unsubscribe<M>
where
    M: Clone + Message<Result = ()>,
{
    type Result = ();
}

impl<M> Handler<Unsubscribe<M>> for RootActor
where
    M: Clone + Message<Result = ()>,
{
    type HandleFuture<'a> = Ready<()>;

    fn handle(
        &mut self,
        message: Unsubscribe<M>,
        _ctx: &mut Context<Self>,
    ) -> Self::HandleFuture<'_> {
        let broker = self.get_or_create_broker::<M>();
        broker.unsubscribe(message.id);

        ready(())
    }
}

pub struct Broadcast<M>(pub M);

impl<M> Message for Broadcast<M>
where
    M: Clone + Message<Result = ()>,
{
    type Result = ();
}

impl<M> Handler<Broadcast<M>> for RootActor
where
    M: Clone + Message<Result = ()>,
{
    type HandleFuture<'a> = Ready<()>;

    fn handle(
        &mut self,
        message: Broadcast<M>,
        _ctx: &mut Context<Self>,
    ) -> Self::HandleFuture<'_> {
        let broker = self.get_or_create_broker::<M>();
        broker.broadcast(message.0);

        ready(())
    }
}
