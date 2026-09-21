//! v0 — patch theory over a live, replayable event graph.
//!
//! Three layers, bottom up:
//!
//! - **capture** — [`event`] holds an append-only graph of [`op`]s, synced
//!   peer-to-peer by [`sync`]. Live editing and version history write here, to
//!   the same store. An event that belongs to no change is ordinary.
//! - **meaning** — [`change`] labels *subsets* of that graph and derives what
//!   each subset actually depends on. This is the only thing patch theory still
//!   has to contribute, and the whole point of the spike.
//! - **materialisation** — [`replay`] folds a dependency-closed subset into a
//!   worktree. Because it reads a set, merge is union and commutation is a
//!   property rather than an operation.
//!
//! [`store`] is the only module that touches the filesystem; everything else is
//! pure and testable in-process, except [`hook`], the only one that starts
//! processes.
pub mod budget;
pub mod capture;
pub mod change;
pub mod conflict;
pub mod event;
pub mod hook;
pub mod op;
pub mod replay;
pub mod repo;
pub mod store;
pub mod sync;
pub mod tree;
