use std::{
    any::Any,
    future::Future,
    ops::ControlFlow,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
};

use futures_util::FutureExt;
use tokio::task::JoinHandle;

use crate::{
    mailbox::MailboxReceiver,
    supervisor::{ActorFate, Supervisor},
    Actor, Address, NewActor,
};

/// The current execution state of an actor.
///
/// This can be queried through an actor's [`Context`] for
/// internal insight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActorState {
    /// The actor is currently in the process of starting up.
    ///
    /// No messages will be processed until the actor enters
    /// [`ActorState::Operating`] after successful invocation
    /// of [`Actor::starting`].
    Starting,
    /// The actor is running and actively processing messages.
    Operating,
    /// The actor is in the process of stopping.
    ///
    /// It currently does not process any messages and its
    /// supervisor is responsible for deciding whether the
    /// actor will transition back to [`ActorState::Starting`]
    /// or go into [`ActorState::Stopped`].
    // TODO: Doc reference to Supervisor once implemented.
    Stopping,
}

impl ActorState {
    /// Indicates whether the actor is currently alive.
    ///
    /// This is an umbrella term for [`ActorState::Starting`]
    /// and [`ActorState::Operating`].
    pub fn alive(&self) -> bool {
        use ActorState::*;
        matches!(self, Starting | Operating)
    }

    /// Indicates whether the actor is currently shutting down.
    ///
    /// This state is the result of an invocation of
    /// [`Context::stop`] and the actor will not accept any more
    /// messages unless it is restarted.
    pub fn stopping(&self) -> bool {
        matches!(self, ActorState::Stopping)
    }
}

// We assume that no reasonable workload will spawn enough actors
// to overflow `u64`. Also, we don't care about *which* IDs actors
// get, we only want them to be unique; `Ordering::Relaxed` suffices.
#[inline]
fn next_actor_id() -> u64 {
    static ID: AtomicU64 = AtomicU64::new(0);
    ID.fetch_add(1, Ordering::Relaxed)
}

/// TODO
#[derive(Debug)]
pub struct Context<A: Actor> {
    id: u64,
    state: ActorState,
    mailbox: MailboxReceiver<A>,
}

impl<A: Actor> Context<A> {
    pub(crate) fn new(mailbox: MailboxReceiver<A>) -> Self {
        Self {
            id: next_actor_id(),
            state: ActorState::Starting,
            mailbox,
        }
    }

    /// Gets the current [`ActorState`] of the actor governed
    /// by this context.
    ///
    /// Since the state of an actor constantly changes, it is
    /// not recommended to cache this value. Instead, it should
    /// be queried through this method again when needed.
    pub fn state(&self) -> ActorState {
        self.state
    }

    /// Gets the unique, numeric identifier associated with the
    /// actor governed by this context.
    ///
    /// The only assumption that is safe to make about an actor's
    /// ID is that no two actors will ever share the same value.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Initiates the shutdown of the actor that is governed by
    /// this context.
    ///
    /// This method will return immediately and no more messages
    /// will be processed after calling it.
    pub fn stop(&mut self) {
        self.state = ActorState::Stopping;
    }

    /// Gets a strong [`Address`] of the actor that is governed by
    /// this context.
    ///
    /// This allows an actor to retrieve shareable references to
    /// itself during message processing.
    pub fn address(&self) -> Address<A> {
        Address::new(self.id, self.mailbox.create_sender())
    }

    // TODO: Support for scheduling messages to the actor.
    // TODO: Streams?
}

// Private implementation of the actor lifecycle and supervision
// logic which makes up the heart of the monroe crate.

struct Runner<S, NA: NewActor> {
    actor: NA::Actor,
    supervisor: S,
    new_actor: NA,
    context: Context<NA::Actor>,
}

impl<S: Supervisor<NA>, NA: NewActor> Runner<S, NA> {
    fn new(
        supervisor: S,
        mut new_actor: NA,
        mut context: Context<NA::Actor>,
        arg: NA::Arg,
    ) -> Result<Self, NA::Error> {
        Ok(Self {
            actor: new_actor.make(&mut context, arg)?,
            supervisor,
            new_actor,
            context,
        })
    }

    fn create_new_actor(&mut self, arg: NA::Arg) -> Result<(), NA::Error> {
        self.new_actor.make(&mut self.context, arg).map(|actor| {
            // Force the actor to be dropped in place while replacing it.
            unsafe { Pin::new_unchecked(&mut self.actor) }.set(actor);
        })
    }

    fn handle_restart_error(&mut self, error: NA::Error) -> ControlFlow<()> {
        match self.supervisor.on_restart_failure(error) {
            ActorFate::Restart(arg) => match self.create_new_actor(arg) {
                Ok(()) => ControlFlow::CONTINUE,
                Err(error) => {
                    // Notify the supervisor that this actor is gone.
                    self.supervisor.on_second_restart_failure(error);

                    ControlFlow::BREAK
                }
            },

            ActorFate::Stop => ControlFlow::BREAK,
        }
    }

    fn handle_stop(&mut self) -> ControlFlow<()> {
        match self.supervisor.on_graceful_stop() {
            ActorFate::Restart(arg) => match self.create_new_actor(arg) {
                Ok(()) => ControlFlow::CONTINUE,
                Err(error) => self.handle_restart_error(error),
            },

            ActorFate::Stop => ControlFlow::BREAK,
        }
    }

    fn handle_panic(&mut self, panic: Box<dyn Any + Send + 'static>) -> ControlFlow<()> {
        match self.supervisor.on_panic(panic) {
            ActorFate::Restart(arg) => match self.create_new_actor(arg) {
                Ok(()) => ControlFlow::CONTINUE,
                Err(error) => self.handle_restart_error(error),
            },

            ActorFate::Stop => ControlFlow::BREAK,
        }
    }
}

impl<A: Actor> Context<A> {
    /// Executes the given [`Actor`] object in this context.
    pub(crate) fn run<S: Supervisor<NA>, NA: NewActor<Actor = A>>(
        self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Arg,
    ) -> Result<JoinHandle<()>, NA::Error> {
        let runner = Runner::new(supervisor, new_actor, self, arg)?;

        // TODO: Runtime abstraction for spawn calls?
        Ok(tokio::spawn(async move {
            /*// Initialize the actor before processing any messages.
            if let Err(panic) = panic_safe(runner.actor.starting(&mut self)).await {
                todo!()
            }

            // The actor is now considered running, if not already stopped.
            if self.state.stopping() {
                todo!()
            }
            self.state = ActorState::Operating;

            // Poll the inbox for new messages to process.
            while let Ok(letter) = self.mailbox.recv().await {
                if let Err(panic) = panic_safe(letter.deliver(&mut actor, &mut self)).await {
                    todo!()
                }

                // Handle the case where a message handler initiated shutdown.
                if self.state.stopping() {
                    todo!()
                }
            }

            // The actor has terminated, perform final cleanup and drop it.
            if let Err(panic) = panic_safe(actor.stopped()).await {
                todo!()
            }*/
        }))
    }
}

#[inline(always)]
async fn panic_safe<Fut: Future<Output = T>, T>(fut: Fut) -> std::thread::Result<T> {
    AssertUnwindSafe(fut).catch_unwind().await
}
