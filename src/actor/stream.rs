use std::future::Future;

use super::Actor;
use crate::Context;

/// Describes how to process streams of items attached to
/// an [`Actor`].
///
/// Unlike [`Handler`][crate::Handler], this does not require
/// stream items to implement the [`Message`][crate::Message]
/// trait.
pub trait StreamHandler<I: Send + 'static>: Actor {
    /// The [`Future`] type produced by [`StreamHandler::started`].
    type StartedFuture<'a>: Future<Output = ()> + Send + 'a
    where
        Self: 'a;

    /// The [`Future`] type produced by [`StreamHandler::handle`].
    type HandleFuture<'a>: Future<Output = ()> + Send + 'a
    where
        Self: 'a,
        I: 'a;

    /// The [`Future`] type produced by [`StreamHandler::finished`].
    type FinishedFuture<'a>: Future<Output = ()> + Send + 'a
    where
        Self: 'a;

    /// Called when a stream is initially attached.
    ///
    /// At this point no stream items have been processed
    /// yet, so this method may be used for initialization
    /// work beforehand.
    fn started(&mut self, ctx: &mut Context<Self>) -> Self::StartedFuture<'_>;

    /// Processes the next item yielded by the stream.
    ///
    /// This is somewhat akin to
    /// [`Handler::handle`][crate::Handler::handle],
    /// except that it must only return unit (`()`).
    ///
    /// While a [`Handler`][crate::Handler] forwards any values
    /// it produces to the caller which takes responsibility
    /// for handling errors and such, a [`StreamHandler`] is
    /// not directly called by anyone capable of fulfilling
    /// this role.
    ///
    /// The implementation should use [`Context::stop`] as an
    /// alternative to returning errors. These stops can be
    /// forwarded to the supervisor.
    ///
    /// When the actor is restarted, no more stream elements
    /// will be processed, just like any other outstanding
    /// messages that were addressed at the old actor.
    fn handle(&mut self, item: I, ctx: &mut Context<Self>) -> Self::HandleFuture<'_>;

    /// Called when the stream has been fully consumed.
    ///
    /// This method assumes natural termination of the stream.
    /// It will not be called if an actor decides to shut down
    /// during handling of a stream item or if a panic is caused.
    ///
    /// Many stream-processing [`Actor`]s have their logic
    /// centered around the processing of these items. As such,
    /// a valid implementation of this method often is to just
    /// call [`Context::stop`].
    fn finished(&mut self, ctx: &mut Context<Self>) -> Self::FinishedFuture<'_>;
}
