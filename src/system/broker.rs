use rustc_hash::FxHashMap;

use crate::{Actor, Handler, Message, TellError, WeakAddress};

pub struct Broker<M: Clone + Message<Result = ()>> {
    subscribers: FxHashMap<u64, Box<dyn Recipient<M>>>,
}

impl<M: Clone + Message<Result = ()>> Broker<M> {
    pub fn subscribe<A>(&mut self, address: WeakAddress<A>)
    where
        A: Actor + Handler<M>,
    {
        self.subscribers.insert(address.id(), Box::new(address));
    }

    pub fn unsubscribe(&mut self, id: u64) {
        self.subscribers.remove(&id);
    }

    pub fn broadcast(&self, message: M) {
        for sub in self.subscribers.values() {
            let _ = sub.send(message.clone());
        }
    }
}

impl<M: Clone + Message<Result = ()>> Default for Broker<M> {
    fn default() -> Self {
        Self {
            subscribers: FxHashMap::default(),
        }
    }
}

trait Recipient<M: Clone + Message<Result = ()>>: Send {
    fn send(&self, message: M) -> Result<(), TellError<M>>;
}

impl<A, M> Recipient<M> for WeakAddress<A>
where
    A: Actor + Handler<M>,
    M: Clone + Message<Result = ()>,
{
    fn send(&self, message: M) -> Result<(), TellError<M>> {
        match self.upgrade() {
            Some(addr) => addr.try_tell(message),
            None => Err(TellError::Disconnected(message)),
        }
    }
}
