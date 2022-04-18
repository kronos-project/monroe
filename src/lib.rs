//! TODO

#![deny(
    missing_docs,
    rust_2018_idioms,
    rustdoc::broken_intra_doc_links,
    unsafe_op_in_unsafe_fn
)]
#![feature(control_flow_enum, generic_associated_types, never_type)]
#![allow(dead_code)] // FIXME: remove once CI passes with this disabled

mod actor;
pub use self::actor::*;

mod address;
pub use self::address::*;

mod context;
pub use self::context::*;

mod mailbox;

pub mod supervisor;

mod system;
pub use self::system::*;

// TODO: Better oneshot channels than tokio::sync::mpsc?
