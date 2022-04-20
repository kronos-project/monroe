use std::{num::NonZeroUsize, sync::Arc};

use uuid::Uuid;

use crate::{
    mailbox,
    supervisor::{NoRestart, Supervisor},
    Actor, Address, Context, NewActor, ROOT_ACTOR_ID,
};

mod handle;
pub use self::handle::*;

mod root;
use self::root::*;

/// The configuration for an [`ActorSystem`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActorSystemConfig {
    /// The mailbox size of an actor.
    ///
    /// By default, actors have unbounded mailboxes and are
    /// capable of receiving and processing arbitrary numbers
    /// of messages.
    ///
    /// More constrained and computationally heavy actors can
    /// opt to use bounded mailbox sizes to not exhaust the
    /// available memory when falling behind on processing
    /// speed for large volumes of received messages.
    pub mailbox_size: Option<NonZeroUsize>,
    /// The mailbox size of the **root actor**.
    ///
    /// The root actor is the central, highest actor in an
    /// application hierarchy. Thus, its mailbox configuration
    /// is separated from ordinary actors. See
    /// [`ActorSystemConfig::mailbox_size`] for those.
    ///
    /// By default, the root actors has unbounded mailboxes
    /// and is capable of receiving and processing arbitrary
    /// numbers of messages.
    pub root_mailbox_size: Option<NonZeroUsize>,
}

/// The anchorpoint to a hierarchy of actors in an application.
///
/// Actor Systems are the heart and soul of every actor-based
/// application - they are responsible for spawning, managing
/// and providing access to actors.
///
/// As a rule per thumb, the highest actors in the application
/// hierarchy are tracked by the system. These actors can then
/// independently form their own hierarchies and spread their
/// work across them.
///
/// At its core, it keeps an address to the **root actor**, the
/// highest actor in the hierarchy which is spawned and managed
/// independently from user-created actors and most operations
/// invoked on the system will lead to communication with said
/// actor.
///
/// A system is exposed through the [`Context`][crate::Context]
/// for all actors -- even those that are not directly tracked
/// in the system. Instead, [`ActorSystem`] handles get passed
/// down from managed actors to unmanaged ones when spawned as
/// subordinates.
#[derive(Debug)]
pub struct ActorSystem {
    inner: Arc<ActorSystemInner>,
}

#[derive(Debug)]
struct ActorSystemInner {
    uuid: Uuid,
    root: Address<RootActor>,
    config: ActorSystemConfig,
}

impl ActorSystem {
    /// Creates a new actor system given a configuration.
    pub fn new(config: ActorSystemConfig) -> Self {
        // Construct the root actor's mailbox based on the given configuration.
        let (sender, receiver) = match config.root_mailbox_size {
            Some(capacity) => mailbox::bounded(capacity.get()),
            None => mailbox::unbounded(),
        };

        // Create the `ActorSystem` that will be shared with all actors.
        let system = Self {
            inner: Arc::new(ActorSystemInner {
                uuid: Uuid::new_v4(),
                root: Address::new(ROOT_ACTOR_ID, sender),
                config,
            }),
        };

        // Spawn the root actor. We don't need to deal with the result
        // because as long as the error type is `!`, it can never fail.
        // TODO: Do we need the join handle?
        let context = Context::new_root(system.clone(), receiver);
        let _ = context.run(NoRestart, RootActor::new as fn(&mut _) -> _, ());

        system
    }

    // Crate-internal clone method so people cannot move out of the
    // `ActorSystem` references they are given by the context. This
    // ensures that methods like `ActorSystem::wait_for_shutdown`
    // are always used as intended.
    pub(crate) fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Gets an immutable reference to the UUID that is
    /// assigned to this system.
    ///
    /// NOTE: UUIDs are randomly generated on system creation
    /// and can be used to uniquely identify systems in a
    /// cluster.
    pub fn uuid(&self) -> &Uuid {
        &self.inner.uuid
    }

    /// Gets an immutable reference to the [`ActorSystemConfig`]
    /// in use.
    pub fn config(&self) -> &ActorSystemConfig {
        &self.inner.config
    }

    ///
    pub async fn spawn<S: Supervisor<NA>, NA: NewActor>(
        &self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Arg,
        capacity: Option<NonZeroUsize>,
    ) -> Result<Address<NA::Actor>, NA::Error> {
        // Construct the actor's execution context.
        let (sender, receiver) = match capacity.or_else(|| self.config().mailbox_size) {
            Some(capacity) => mailbox::bounded(capacity.get()),
            None => mailbox::unbounded(),
        };
        let context = Context::new(self.clone(), receiver);
        let address = Address::new(context.id(), sender);

        // Spawn the actor onto the runtime and store its handle.
        let _ = self
            .inner
            .root
            .ask(AddActor(ActorHandle {
                address: address.clone(),
                handle: context.run(supervisor, new_actor, arg)?,
            }))
            .await;

        Ok(address)
    }

    ///
    pub async fn find_actor<A: Actor>(&self, id: u64) -> Option<Address<A>> {
        self.inner.root.ask(FindActor::new(id)).await.unwrap()
    }

    ///
    pub async fn wait_for_shutdown(self) {
        // Request all the task handles currently stored in the root actor.
        let handles = self.inner.root.ask(DrainActors).await.unwrap();

        // Wait for all these tasks to terminate.
        for handle in handles.into_iter() {
            let _ = handle.await;
        }
    }
}
