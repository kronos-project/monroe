mod envelope;
pub use self::envelope::*;

// TODO: The end goal is not having to allocate every `EnvelopeProxy::deliver`
//       future on the heap. This requires us to assign a GAT to the trait
//       which currently breaks object-safety: https://github.com/rust-lang/rust/issues/81823
//
//       Once we can, switch to a pin-projected future type that unifies
//       `ForgettingEnvelope`, `ReturningEnvelope` and `RequestingEnvelope`
//       into a unified future type.
//
//       We can then use something along the lines of
//       `type Letter<A> = Box<dyn for<'a> EnvelopeProxy<Actor = A, Future<'a> = ProxyFuture<...>>>`
//       in our mailboxes for minimal memory footprint.
