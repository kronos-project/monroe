mod envelope;
pub use self::envelope::*;

mod channel;

// TODO: The end goal is not having to allocate every `EnvelopeProxy::deliver`
//       future on the heap. This requires us to assign a GAT to the trait
//       which currently breaks object-safety: https://github.com/rust-lang/rust/issues/81823
//
//       Once we can, switch to a pin-projected future type that unifies
//       `ForgettingEnvelope`, `ReturningEnvelope` and `RequestingEnvelope`
//       into a common return type.
//
//       We can then use something along the lines of
//       `Box<dyn for<'a> EnvelopeProxy<Actor = A, Future<'a> = ProxyFuture<...>>>`
//       in our mailboxes for minimal memory footprint.
pub type Letter<A> = Box<dyn EnvelopeProxy<Actor = A>>;

pub type MailboxSender<A> = channel::Sender<Letter<A>>;
pub type WeakMailboxSender<A> = channel::WeakSender<Letter<A>>;
pub type MailboxReceiver<A> = channel::Receiver<Letter<A>>;

pub type OneshotSender<T> = tokio::sync::oneshot::Sender<T>;
pub type OneshotReceiver<T> = tokio::sync::oneshot::Receiver<T>;
