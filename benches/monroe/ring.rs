use crate::RingSpec;
use monroe::{supervisor::NoRestart, Actor, ActorSystem, Address, Context, Handler, Message};
use std::future::{ready, Future, Ready};

pub async fn run(spec: RingSpec) {
    let sys = ActorSystem::new(Default::default());

    let first = sys
        .spawn(
            NoRestart,
            Ring::new as fn(&mut _, _, _) -> _,
            (0u32, spec),
            None,
        )
        .await
        .unwrap();
    let first2 = first.clone();

    first.tell(CloseRing { first: first2 }).await.unwrap();

    first.tell(Payload(0)).await.unwrap();

    sys.wait_for_shutdown().await;
}

#[derive(Debug)]
pub struct Payload(u32);

impl Message for Payload {
    type Result = ();
}

#[derive(Debug, Clone)]
pub struct Stop;

impl Message for Stop {
    type Result = ();
}

#[derive(Debug)]
pub struct CloseRing {
    first: Address<Ring>,
}
impl Message for CloseRing {
    type Result = ();
}

pub struct Ring {
    next: Option<Address<Self>>,
    id: u32,
    spec: RingSpec,
}

impl Ring {
    pub fn new(_: &mut Context<Self>, id: u32, spec: RingSpec) -> Self {
        Self {
            next: None,
            id,
            spec,
        }
    }
}

impl Actor for Ring {
    type StartingFuture<'a> = impl Future<Output = ()> + 'a;
    type StoppedFuture = Ready<()>;

    fn starting<'a>(&'a mut self, ctx: &'a mut Context<Self>) -> Self::StartingFuture<'a> {
        async move {
            ctx.subscribe::<Stop>().await;

            if self.id >= self.spec.nodes {
                return;
            }

            self.next = Some(
                ctx.spawn_actor(
                    NoRestart,
                    Ring::new as fn(&mut _, _, _) -> _,
                    (self.id + 1, self.spec),
                    None,
                )
                .unwrap()
                .address(),
            );
        }
    }

    fn stopped(&mut self) -> Self::StoppedFuture {
        ready(())
    }
}

impl Handler<Payload> for Ring {
    type HandleFuture<'a> = impl Future<Output = ()> + 'a;

    fn handle<'a>(
        &'a mut self,
        Payload(msg): Payload,
        ctx: &'a mut Context<Self>,
    ) -> Self::HandleFuture<'a> {
        async move {
            if msg >= self.spec.limit {
                ctx.system().broker_broadcast(Stop).await;
                return;
            }

            self.next
                .as_ref()
                .unwrap()
                .tell(Payload(msg + 1))
                .await
                .unwrap();
        }
    }
}

impl Handler<CloseRing> for Ring {
    type HandleFuture<'a> = impl Future<Output = ()>;

    fn handle(&mut self, message: CloseRing, _: &mut Context<Self>) -> Self::HandleFuture<'_> {
        async move {
            match self.next {
                Some(ref next) => {
                    next.tell(message).await.unwrap();
                }
                None => self.next = Some(message.first),
            }
        }
    }
}

impl Handler<Stop> for Ring {
    type HandleFuture<'a> = Ready<()>;

    fn handle<'a>(&'a mut self, _: Stop, ctx: &'a mut Context<Self>) -> Self::HandleFuture<'a> {
        ctx.stop();
        ready(())
    }
}
