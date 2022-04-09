use std::sync::atomic::{AtomicU64, Ordering};

use crate::{mailbox::MailboxReceiver, Actor, Address};

/// The current execution state of an actor.
///
/// This can be queried through an actor's [`Context`] for
/// internal insight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActorState {
    /// The actor is currently in the process of starting up.
    ///
    /// No messages will be processed until the actor enters
    /// [`ActorState::Operating`].
    Starting,
    /// The actor is running and actively processes messages.
    Operating,
    /// The actor is in the process of stopping.
    ///
    /// It currently does not process any messages and its
    /// supervisor is responsible for deciding whether the
    /// actor will transition back to [`ActorState::Starting`]
    /// or go into [`ActorState::Stopped`].
    // TODO: Doc reference to Supervisor once implemented.
    Stopping,
    /// The actor has ultimately terminated and will never be
    /// restarted again.
    Stopped,
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
