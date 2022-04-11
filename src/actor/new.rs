use super::Actor;
use crate::Context;

/// Unified interface to describe the creation of new [`Actor`]s
/// throughout the entire crate.
pub trait NewActor: Send + 'static {
    /// The [`Actor`] type produced by this factory.
    type Actor: Actor;

    /// The argument type that is passed to [`NewActor::make`]
    /// for creating new actors.
    ///
    /// For the initial instance, the argument's value must be
    /// provided by the user. For subsequent restarts of failed
    /// actors, the arguments are obtained from the actor's
    /// [`Supervisor`][crate::supervisor::Supervisor].
    type Arg;

    /// The error type eventually produced by [`NewActor::make`].
    ///
    /// It is valid to use the [never type] (`!`) to indicate
    /// that the creation of actors may never fail.
    ///
    /// [never type]: https://doc.rust-lang.org/std/primitive.never.html
    type Error;

    /// Attempts to create a new [`Actor`] using the supplied
    /// argument.
    ///
    /// Failures to create new actors will be forwarded to
    /// `Supervisor::on_restart_failure` or
    /// `Supervisor::on_second_restart_failure`.
    fn make(
        &mut self,
        ctx: &mut Context<Self::Actor>,
        arg: Self::Arg,
    ) -> Result<Self::Actor, Self::Error>;
}

// TODO: Implementation that leverages Default trait impls.
