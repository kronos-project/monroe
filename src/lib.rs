//! TODO

#![deny(
    missing_docs,
    rust_2018_idioms,
    rustdoc::broken_intra_doc_links,
    unsafe_op_in_unsafe_fn
)]
#![feature(control_flow_enum, generic_associated_types)]

mod actor;
pub use self::actor::*;

mod address;
pub use self::address::*;

mod context;
pub use self::context::*;

mod mailbox;

pub mod supervisor;

// TODO: Better oneshot channels than tokio::sync::mpsc?
