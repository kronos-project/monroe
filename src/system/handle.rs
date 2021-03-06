use std::{
    fmt,
    future::Future,
    ops::Deref,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};

pub use tokio::task::JoinError;
use tokio::task::JoinHandle;

#[allow(unused_imports)] // We want Context in scope for docs.
use crate::{Actor, Address, Context};

/// An owning handle to an actor's task.
///
/// These handles can be obtained from when creating a child
/// actor with [`Context::spawn_actor`].
///
/// Handles carry a strongly reference-counted [`Address`] for
/// their actor and are also capable of aborting or waiting for
/// an actor's task on the runtime to complete.
///
/// Actors that are directly tracked by an
/// [`ActorSystem`][crate::ActorSystem] will not have their
/// own handles managed by that instead of having them exposed
/// for handling by another actor.
pub struct ActorHandle<A: Actor> {
    pub(crate) address: Address<A>,
    pub(crate) handle: JoinHandle<()>,
}

impl<A: Actor> ActorHandle<A> {
    /// Aborts the actor's task in its current form.
    ///
    /// Awaiting the aborted handle may or may not complete
    /// successfully if the task has already joined, but will
    /// produce [`JoinError`] in most cases.
    #[inline]
    pub fn abort(&self) {
        self.handle.abort()
    }

    /// Creates a new strong [`Address`] for the actor.
    ///
    /// The returned object will cause the reference count
    /// for the actor lifecycle to be incremented.
    #[inline]
    pub fn address(&self) -> Address<A> {
        self.address.clone()
    }
}

impl<A: Actor> fmt::Debug for ActorHandle<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorHandle")
            .field("address", &self.address)
            .finish()
    }
}

impl<A: Actor> Deref for ActorHandle<A> {
    type Target = Address<A>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.address
    }
}

impl<A: Actor> From<ActorHandle<A>> for Address<A> {
    #[inline]
    fn from(handle: ActorHandle<A>) -> Self {
        handle.address
    }
}

impl<A: Actor> From<ActorHandle<A>> for JoinHandle<()> {
    #[inline]
    fn from(handle: ActorHandle<A>) -> Self {
        handle.handle
    }
}

/// Waits for the completion of the actor task.
impl<A: Actor> Future for ActorHandle<A> {
    type Output = Result<(), JoinError>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut this.handle).poll(cx)
    }
}
