//! v0 — patch theory over a CRDT op log.
//!
//! The layering, top to bottom:
//!
//! - [`repo`]  — the four operations a user performs, all of them set algebra
//! - [`change`] — a *named set* of ops plus its derived minimal dependencies
//! - [`weave`]  — the materialiser `M(S)`: a dependency-closed set to a document
//! - [`op`]     — the atoms of edit: an insert or a delete, each with an identity
//!
//! `store` is the only module that touches the filesystem; everything above it
//! is pure and testable without a process.
pub mod change;
pub mod op;
pub mod repo;
pub mod store;
pub mod weave;
