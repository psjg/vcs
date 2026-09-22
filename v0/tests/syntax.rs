//! The structure layer against its oracles: a fresh parse, and replay.
#![allow(unused_imports)] // skeleton

use v0::syntax::{Grammar, Syntax};

/// After any sequence of edits -- inserts, deletes, moves, in any causal
/// order -- the tree followed incrementally equals a fresh parse of the
/// weave's text: same S-expression, same changed ranges accounted for.
#[test]
#[ignore = "skeleton"]
fn a_followed_tree_equals_a_fresh_parse() {
    todo!()
}

/// A ref names the same node through edits around it, and says so when the
/// node no longer parses as one.
#[test]
#[ignore = "skeleton"]
fn a_node_ref_survives_edits_around_its_node() {
    todo!()
}

/// The acceptance test for intents: Alice renames `foo` to `bar` everywhere;
/// Bob, concurrently, adds a call to `foo`. After the merge Bob's call reads
/// `bar`, on both replicas, and merging the same histories twice mints no
/// second rename of it.
#[test]
#[ignore = "skeleton"]
fn a_rename_reaches_a_reference_added_concurrently() {
    todo!()
}
