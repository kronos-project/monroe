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

macro_rules! impl_new_actor_for_fn {
    ($(($($name:ident: $ty:ident),*)),* $(,)*) => {
        $(
            impl<A, $($ty),*> NewActor for fn(&mut Context<A>, $($ty),*) -> A
            where
                A: Actor,
                $($ty: 'static,)*
            {
                type Actor = A;
                type Arg = ($($ty),*);
                type Error = !;

                #[inline]
                fn make(
                    &mut self,
                    ctx: &mut Context<Self::Actor>,
                    arg: Self::Arg,
                ) -> Result<Self::Actor, Self::Error> {
                    let ($($name),*) = arg;
                    Ok(self(ctx, $($name),*))
                }
            }

            impl<A, E, $($ty),*> NewActor for fn(&mut Context<A>, $($ty),*) -> Result<A, E>
            where
                A: Actor,
                E: 'static,
                $($ty: 'static,)*
            {
                type Actor = A;
                type Arg = ($($ty),*);
                type Error = E;

                #[inline]
                fn make(
                    &mut self,
                    ctx: &mut Context<Self::Actor>,
                    arg: Self::Arg,
                ) -> Result<Self::Actor, Self::Error> {
                    let ($($name),*) = arg;
                    self(ctx, $($name),*)
                }
            }
        )*
    };
}

impl<A, Arg> NewActor for fn(&mut Context<A>, Arg) -> A
where
    A: Actor,
    Arg: 'static,
{
    type Actor = A;
    type Arg = Arg;
    type Error = !;

    #[inline]
    fn make(
        &mut self,
        ctx: &mut Context<Self::Actor>,
        arg: Self::Arg,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(self(ctx, arg))
    }
}

impl<A, Arg, E> NewActor for fn(&mut Context<A>, Arg) -> Result<A, E>
where
    A: Actor,
    Arg: 'static,
    E: 'static,
{
    type Actor = A;
    type Arg = Arg;
    type Error = E;

    #[inline]
    fn make(
        &mut self,
        ctx: &mut Context<Self::Actor>,
        arg: Self::Arg,
    ) -> Result<Self::Actor, Self::Error> {
        self(ctx, arg)
    }
}

impl_new_actor_for_fn! {
    (),
    // -- Manually implemented for one arg to avoid tuple representation. --
    (a1: A1, a2: A2),
    (a1: A1, a2: A2, a3: A3),
    (a1: A1, a2: A2, a3: A3, a4: A4),
    (a1: A1, a2: A2, a3: A3, a4: A4, a5: A5),
    (a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6),
    (a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7),
    (a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8),
    (a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9),
    (a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9, a10: A10),
}
