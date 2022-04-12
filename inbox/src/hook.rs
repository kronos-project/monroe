use std::sync::Arc;

pub type Spinlock<T> = spin::Mutex<T>;

/// A [`SenderHook`] represents the signal that a waiting
/// sender will insert into the `sending` list.
pub struct SenderHook<T, S: ?Sized> {
    pub(crate) slot: Spinlock<Option<T>>,
    pub(crate) signal: S,
}

impl<T, S: ?Sized> SenderHook<T, S> {
    pub fn new(msg: Option<T>, signal: S) -> Arc<Self>
    where
        S: Sized,
    {
        Arc::new(Self {
            slot: Spinlock::new(msg),
            signal,
        })
    }

    pub fn signal(&self) -> &S {
        &self.signal
    }

    pub fn take_msg(&self) -> (T, &S) {
        let msg = self.slot.lock().take().unwrap();
        (msg, self.signal())
    }

    pub fn is_empty(&self) -> bool {
        self.slot.lock().is_none()
    }

    pub fn try_take(&self) -> Option<T> {
        self.slot.lock().take()
    }
}

/// A hook can either be a trigger or slot one.
///
/// **Trigger** hooks do not carry any data and will only
/// fire a signal once.
///
/// **Slot** hooks will fire a signal, but also store the message
/// inside them so it can be taken out later.
pub enum HookKind<T> {
    Slot(Spinlock<Option<T>>),
    Trigger,
}

/// A [`ReceiverHook`] represents the signal that a waiting
/// receiver will insert into the `waiting` list.
pub struct ReceiverHook<T, S: ?Sized> {
    slot: HookKind<T>,
    signal: S,
}

impl<T, S: ?Sized> ReceiverHook<T, S> {
    pub fn signal(&self) -> &S {
        &self.signal
    }

    /// This method will try to send the given message into this
    /// receiver hook.
    ///
    /// If this hook is a [`HookKind::Slot`], `None` is returned, indicating that
    /// this hook consumed this message.
    ///
    /// Otherwise, `Some` with the original message is returned, indicating that
    /// the corresponding receiver will take the message out of the message
    /// queue when it signal fires.
    pub fn send(&self, msg: T) -> Option<T> {
        match self.slot {
            HookKind::Slot(slot) => {
                *slot.lock() = Some(msg);
                None
            }
            HookKind::Trigger => Some(msg),
        }
    }
}
