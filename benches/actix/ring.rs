use crate::RingSpec;
use actix::prelude::*;

pub fn run(spec: RingSpec) {
    let sys = System::new();

    sys.block_on(async {
        let first = RingNode {
            id: 0,
            spec,
            next: None,
        }
        .start();

        let first2 = first.clone();
        first.send(CloseRing { first: first2 }).await.unwrap();
        first.send(Payload(0)).await.unwrap();
    });

    sys.run().unwrap();
}

#[derive(Debug, Message)]
#[rtype(result = "()")]
pub struct Payload(u32);

#[derive(Debug, Message)]
#[rtype(result = "()")]
pub struct CloseRing {
    first: Addr<RingNode>,
}

#[derive(Debug)]
struct RingNode {
    id: u32,
    spec: RingSpec,
    next: Option<Addr<RingNode>>,
}

impl Actor for RingNode {
    type Context = Context<Self>;

    fn started(&mut self, _: &mut Self::Context) {
        if self.id >= self.spec.nodes {
            return;
        }

        self.next = Some(
            RingNode {
                id: self.id + 1,
                spec: self.spec,
                next: None,
            }
            .start(),
        );
    }
}

impl Handler<Payload> for RingNode {
    type Result = ();

    fn handle(&mut self, msg: Payload, _: &mut Context<Self>) {
        if msg.0 >= self.spec.limit {
            System::current().stop();
            return;
        }

        self.next.as_ref().unwrap().do_send(Payload(msg.0 + 1));
    }
}

impl Handler<CloseRing> for RingNode {
    type Result = ();

    fn handle(&mut self, msg: CloseRing, _: &mut Context<Self>) {
        match self.next {
            Some(ref next) => next.do_send(msg),
            None => self.next = Some(msg.first),
        }
    }
}
