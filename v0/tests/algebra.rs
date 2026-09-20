//! The deliverable: the invariants from DESIGN.md as executable properties.
//!
//! These are `#[ignore]`d until the core is implemented, so the suite stays
//! green and the intent is already reviewable. Remove the attribute as each
//! becomes real.

#[test]
#[ignore = "skeleton"]
fn i1_materialise_is_a_pure_function_of_the_set() {}

#[test]
#[ignore = "skeleton"]
fn i2_merge_is_commutative_associative_and_idempotent() {}

#[test]
#[ignore = "skeleton"]
fn i3_drop_leaves_other_change_ids_byte_identical() {}

#[test]
#[ignore = "skeleton"]
fn i4_changeset_cannot_be_constructed_unclosed() {}

#[test]
#[ignore = "skeleton"]
fn i5_adopt_after_drop_restores_the_original_set() {}

/// The one expected to fail. RGA-family orderings interleave in some concurrent
/// cases; Fugue is the ordering that provably does not. A red result here is a
/// finding, not a bug.
#[test]
#[ignore = "skeleton"]
fn i6_concurrent_insert_blocks_never_interleave() {}

#[test]
#[ignore = "skeleton"]
fn i7_drop_only_touches_lines_the_change_touched() {}
