//! TODO

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]
#![feature(generic_associated_types, type_alias_impl_trait)]
#![forbid(unsafe_code)]

mod actor;
pub use self::actor::*;

mod context;
pub use self::context::*;

mod mailbox;

// TODO: Better oneshot channels than tokio::sync::mpsc?
