//! TODO

use std::{self, any::Any};

use crate::NewActor;

/// The fate of a faulting actor, determined by its [`Supervisor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActorFate<Arg> {
    /// The actor should be restarted, with a user-provided argument
    /// for [`NewActor::make`].
    Restart(Arg),
    /// The actor should be permanently stopped.
    Stop,
}

/// A supervisor that oversees the execution cycle of an
/// [`Actor`][crate::Actor].
///
/// In order to build effective self-healing systems, every
/// actor in monroe must have a designated supervisor which
/// deals with different kinds of errors and crashes an
/// actor by itself cannot recover from.
pub trait Supervisor<NA: NewActor> {
    /// Called when an actor is gracefully shutting down after
    /// calling [`Context::stop`][crate::Context::stop].
    fn on_graceful_stop(&mut self) -> ActorFate<NA::Arg>;

    /// Called when restarting an actor failed for the *first*
    /// time.
    ///
    /// Refer to [`Supervisor::on_second_restart_failure`] for
    /// details on how actors which fail to be restarted for
    /// the *second* time are dealt with.
    ///
    /// # Strategy
    ///
    /// This method is generally invoked when the original
    /// instance of the supervised actor crashed.
    ///
    /// One of the other methods on the [`Supervisor`] trait will
    /// be called first to determine an [`ActorFate`] based on
    /// the cause of said crash.
    ///
    /// When such a method then produces [`ActorFate::Restart`]
    /// but [`NewActor::make`] fails with the argument obtained
    /// from it, this method will be called to deal with the
    /// issue.
    ///
    /// Thus, this method should decide whether it is appropriate
    /// to re-attempt the creation of another actor object.
    fn on_restart_failure(&mut self, error: NA::Error) -> ActorFate<NA::Arg>;

    /// Called when restarting an actor failed for the *second*
    /// time.
    ///
    /// This is called when an actor couldn't be successfully
    /// restarted with [`Supervisor::on_restart_failure`].
    ///
    /// # Strategy
    ///
    /// When re-creating a new actor object after its crash fails
    /// two times in a row, it is likely that a logic bug is the
    /// cause of this.
    ///
    /// In order to protect ourselves from forming a cycle of
    /// ever failing [`NewActor::make`] calls and supervisors
    /// instructing more restarts, this method merely exists to
    /// report the error for the restarting failures.
    ///
    /// It purposefully returns no [`ActorFate`] because actors
    /// will be unconditionally stopped when landing here.
    fn on_second_restart_failure(&mut self, error: NA::Error);

    /// Called when an actor crashed due to a panic in any part
    /// of the actor implementation.
    ///
    /// The default implementation of this method will stop the
    /// actor unconditionally as it is generally harder to recover
    /// from this state of metastability than from normal errors.
    fn on_panic(&mut self, panic: Box<dyn Any + Send + 'static>) -> ActorFate<NA::Arg> {
        drop(panic);
        ActorFate::Stop
    }
}

// TODO: Implementations for closures for more ergonomics.

/// A general-purpose supervisor implementation that never
/// restarts a crashed actor.
#[derive(Debug)]
pub struct NoRestart;

// TODO: Error logging?
impl<NA: NewActor> Supervisor<NA> for NoRestart {
    fn on_graceful_stop(&mut self) -> ActorFate<NA::Arg> {
        ActorFate::Stop
    }

    fn on_restart_failure(
        &mut self,
        _error: <NA as NewActor>::Error,
    ) -> ActorFate<<NA as NewActor>::Arg> {
        ActorFate::Stop
    }

    fn on_second_restart_failure(&mut self, _error: <NA as NewActor>::Error) {}
}
