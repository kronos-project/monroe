use futures_util::future::{ready, Ready};

use crate::{Actor, Context, ROOT_ACTOR_ID};

#[derive(Debug)]
pub struct RootActor {}

impl RootActor {
    pub fn new(ctx: &mut Context<Self>) -> Self {
        debug_assert_eq!(ctx.id(), ROOT_ACTOR_ID);

        Self {}
    }
}

// TODO
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
