use rustc_hash::FxHashMap;

use crate::{Actor, Handler, Message, WeakAddress};

pub struct Broker<M: Clone + Message<Result = ()>> {
    subscribers: FxHashMap<u64, Box<dyn Recipient<M>>>,
}

impl<M: Clone + Message<Result = ()>> Broker<M> {
    #[inline]
    pub fn subscribe<A>(&mut self, address: WeakAddress<A>)
    where
        A: Actor + Handler<M>,
    {
        self.subscribers.insert(address.id(), Box::new(address));
    }

    #[inline]
    pub fn unsubscribe(&mut self, id: u64) {
        self.subscribers.remove(&id);
    }

    #[inline]
    pub fn broadcast(&self, message: M) {
        for sub in self.subscribers.values() {
            sub.send(message.clone());
        }
    }
}

impl<M: Clone + Message<Result = ()>> Default for Broker<M> {
    #[inline]
    fn default() -> Self {
        Self {
            subscribers: FxHashMap::default(),
        }
    }
}

trait Recipient<M: Clone + Message<Result = ()>>: Send {
    fn send(&self, message: M);
}

impl<A, M> Recipient<M> for WeakAddress<A>
where
    A: Actor + Handler<M>,
    M: Clone + Message<Result = ()>,
{
    #[inline]
    fn send(&self, message: M) {
        if let Some(addr) = self.upgrade() {
            let _ = addr.try_tell(message);
        }
    }
}
