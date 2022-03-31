use std::future::Future;

// TODO: Actor context.
// TODO: Supervision.

/// TODO
pub trait Actor: Sized + Send + 'static {
    /// The [`Future`] type produced by [`Actor::starting`].
    type StartingFuture<'a>: Future<Output = ()> + 'a
    where
        Self: 'a;

    /// The [`Future`] type produced by [`Actor::stopping`].
    type StoppedFuture: Future<Output = ()>;

    /// Called when an actor is in the process of starting up.
    ///
    /// During the execution of this method, no messages will
    /// be processed yet. Should be used for initialization
    /// work.
    fn starting(&mut self) -> Self::StartingFuture<'_>;

    /// Called when an actor has been permanently stopped.
    ///
    /// This consumes the actor object and will be responsible
    /// for dropping it. Should be used for final cleanup work.
    fn stopped(self) -> Self::StoppedFuture;
}
