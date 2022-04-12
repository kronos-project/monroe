use crate::{Sender, Shared};
use std::{
    fmt,
    sync::{atomic::Ordering, Arc},
};

/// The receiving end of a channel.
pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Receiver<T> {
    /// Asynchronously receive a value from the channel, returning an error if all senders have been
    /// dropped. If the channel is empty, the returned future will yield to the async runtime.
    pub fn recv(&self) -> RecvFut<'_, T> {
        todo!()
        //RecvFut::new(self)
    }

    /// Creates a new [`Sender`] that will send values to this receiver.
    pub fn create_sender(&self) -> Sender<T> {
        self.shared.sender_count.fetch_add(1, Ordering::Relaxed);
        Sender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // this is the only receiver, so disconnect channel
        self.shared.disconnect_all();
    }
}
