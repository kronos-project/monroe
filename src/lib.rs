//! TODO

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]
#![feature(control_flow_enum, generic_associated_types)]
//#![forbid(unsafe_code)]

mod actor;
pub use self::actor::*;

mod address;
pub use self::address::*;

mod context;
pub use self::context::*;

mod mailbox;

pub mod supervisor;

// TODO: Better oneshot channels than tokio::sync::mpsc?
