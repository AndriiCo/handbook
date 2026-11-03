//! Shared vocabulary for the handbook subsystems: the closed fact-topic
//! enum, the agent interface (request/response types plus the
//! [`ModelClient`](model::ModelClient) trait), and the single error type
//! every fallible subsystem API returns.
//!
//! Lead-owned: changes here ripple to every crate, so edits go through
//! the parent, never parallel tracks.

pub mod error;
pub mod model;
pub mod topics;

pub use error::{HandbookError, Result};
