use futures_util::future::{ready, Ready};

use crate::{Actor, Context};

pub struct SystemActor {}

// TODO
impl Actor for SystemActor {
    type StartingFuture<'a> = Ready<()>;
    type StoppedFuture = Ready<()>;

    fn starting(&mut self, _ctx: &mut Context<Self>) -> Self::StartingFuture<'_> {
        ready(())
    }

    fn stopped(&mut self) -> Self::StoppedFuture {
        ready(())
    }
}
