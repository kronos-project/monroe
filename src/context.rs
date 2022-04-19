use std::{
    any::Any,
    future::Future,
    num::NonZeroUsize,
    ops::ControlFlow,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
};

use futures_util::{future::CatchUnwind, FutureExt};
use tokio::task::JoinHandle;

use crate::{
    mailbox::{self, MailboxReceiver},
    supervisor::{ActorFate, Supervisor},
    system::ActorSystem,
    Actor, ActorHandle, Address, NewActor,
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
    /// It currently does not process any new messages and waits
    /// for its runtime to detect the stopping state.
    ///
    /// [`Supervisor::on_graceful_stop`] will be queried for a
    /// subsequent actor running strategy. If the supervisor
    /// choses to restart the actor, it will transition back
    /// to [`ActorState::Starting`] or else remain in its
    /// current state until ultimately dropped.
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

/// The ID of the **root actor** in an [`ActorSystem`].
pub const ROOT_ACTOR_ID: u64 = 0;

// We assume that no reasonable workload will spawn enough actors
// to overflow `u64`. Also, we don't care about *which* IDs actors
// get, we only want them to be unique; `Ordering::Relaxed` suffices.
#[inline]
fn next_actor_id() -> u64 {
    static ID: AtomicU64 = AtomicU64::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

/// The execution context of an [`Actor`] type.
///
/// Each actor executes within its own context. It is exposed to
/// its actor in [`Actor::starting`] and [`Handler::handle`].
/// Actors may use it to query state or actively influence their
/// own execution.
///
/// [`Handler::handle`]: crate::Handler::handle
#[derive(Debug)]
pub struct Context<A: Actor> {
    id: u64,
    system: ActorSystem,
    mailbox: MailboxReceiver<A>,
    state: ActorState,
}

impl<A: Actor> Context<A> {
    pub(crate) fn new(system: ActorSystem, mailbox: MailboxReceiver<A>) -> Self {
        Self::new_with_id(next_actor_id(), system, mailbox)
    }

    // A specialized method for constructing the root actor with its correct ID.
    pub(crate) fn new_root(system: ActorSystem, mailbox: MailboxReceiver<A>) -> Self {
        Self::new_with_id(ROOT_ACTOR_ID, system, mailbox)
    }

    fn new_with_id(id: u64, system: ActorSystem, mailbox: MailboxReceiver<A>) -> Self {
        Self {
            id,
            system,
            mailbox,
            state: ActorState::Starting,
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

    /// Gets an immutable reference to the [`ActorSystem`] bound
    /// to the context.
    ///
    /// Note that the returned system is the one that anchors the
    /// entire hierarchy that the actor governed by this context
    /// is part of. This does not imply that the actor is directly
    /// tracked by the system.
    pub fn system(&self) -> &ActorSystem {
        &self.system
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

    /// Spawns a child actor to the actor governed by this
    /// context and returns its [`ActorHandle`].
    ///
    /// While child actors also expose the [`ActorSystem`]
    /// associated with the parent actor, they are not directly
    /// tracked in the system.
    ///
    /// Actors are usually responsible for managing the handles
    /// of their children in accordance with its own behavior.
    ///
    /// The `capacity` argument may be used as an optional
    /// override for an actor's mailbox capacity in contrast to
    /// global system configuration.
    pub fn spawn_actor<S: Supervisor<NA>, NA: NewActor>(
        &self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Arg,
        capacity: Option<NonZeroUsize>,
    ) -> Result<ActorHandle<NA::Actor>, NA::Error> {
        let (sender, receiver) = match capacity.or_else(|| self.system.config().mailbox_size) {
            Some(capacity) => mailbox::bounded(capacity.get()),
            None => mailbox::unbounded(),
        };
        let context = Context::new(self.system.clone(), receiver);

        Ok(ActorHandle {
            address: Address::new(context.id, sender),
            handle: context.run(supervisor, new_actor, arg)?,
        })
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
            // Reset the context back into a sane state for the new actor.
            self.context.reset();

            // SAFETY: `self.actor` never moves. We only call this code
            // after moving the runner object into a separate task in
            // `Context::run` where it will permanently stay.

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

    async fn main_loop(&mut self) -> ControlFlow<()> {
        'main: loop {
            // When actors deal with errors/shutdown, we use `ControlFlow` to indicate
            // how to proceed. For `Break`, the actor is permanently shut down and no
            // longer participates in the main loop. `Continue` is our indication of
            // when an actor was restarted. This is to be taken quite literally - instead
            // of taking the place of its predecessor, we begin the main loop from scratch.

            // Initialize the actor before processing any messages.
            if let Err(panic) = panic_safe(self.actor.starting(&mut self.context)).await {
                self.handle_panic(panic)?;
                continue 'main;
            }

            // If the actor has not already been stopped, it is now considered operating.
            if !self.context.state.stopping() {
                self.context.state = ActorState::Operating;
            } else {
                self.handle_stop()?;
                continue 'main;
            }

            // Poll the inbox for new messages to process until no senders are left.
            while let Ok(letter) = self.context.mailbox.recv().await {
                if let Err(panic) =
                    panic_safe(letter.deliver(&mut self.actor, &mut self.context)).await
                {
                    self.handle_panic(panic)?;
                    continue 'main;
                }

                // Handle the case where a message handler initiated shutdown.
                if self.context.state.stopping() {
                    self.handle_stop()?;
                    continue 'main;
                }
            }

            // The actor has reached the end of its lifecycle, we're done here.
            // All `Address`es are dropped so we don't need to restart.
            break ControlFlow::BREAK;
        }
    }
}

impl<A: Actor> Context<A> {
    // Resets the context back into a state reusable for restarted actors.
    // This should take care of resetting all the state mutable by the actor.
    // This includes everything but mailbox and actor ID.
    fn reset(&mut self) {
        self.state = ActorState::Starting;
        // TODO: Clean outstanding messages from the mailbox.
    }

    /// Executes the given [`Actor`] object in this context.
    pub(crate) fn run<S: Supervisor<NA>, NA: NewActor<Actor = A>>(
        self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Arg,
    ) -> Result<JoinHandle<()>, NA::Error> {
        let mut runner = Runner::new(supervisor, new_actor, self, arg)?;

        // TODO: Runtime abstraction for spawn calls?
        Ok(tokio::spawn(async move {
            // Run the main loop until it terminates. We never care
            // about the `ControlFlow::Break` values we're getting.
            let _ = runner.main_loop().await;

            // Consume the actor and perform final cleanup.
            runner.actor.stopped().await;
        }))
    }
}

#[inline(always)]
fn panic_safe<Fut: Future>(fut: Fut) -> CatchUnwind<AssertUnwindSafe<Fut>> {
    AssertUnwindSafe(fut).catch_unwind()
}
