use futures_util::future::{ready, Ready};

use crate::{Actor, Context};

#[derive(Debug)]
pub struct RootActor {}

impl RootActor {
    pub fn new(_ctx: &mut Context<Self>) -> Self {
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
