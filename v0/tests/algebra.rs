//! The deliverable: DESIGN.md's invariants as executable properties.
//!
//! `#[ignore]`d until the core is implemented, so the suite stays green while
//! the intent is already reviewable. Drop the attribute as each becomes real.

// --- the algebra ------------------------------------------------------------

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

/// Expected to fail. RGA-family orderings interleave in some concurrent cases;
/// Fugue is the ordering that provably does not. Red here is a finding.
#[test]
#[ignore = "skeleton"]
fn i6_concurrent_insert_blocks_never_interleave() {}

#[test]
#[ignore = "skeleton"]
fn i7_drop_only_perturbs_lines_the_change_touched() {}

// --- the reduction: what the spike exists to answer -------------------------

#[test]
#[ignore = "skeleton"]
fn i8_minimal_deps_are_a_subset_of_causal_deps() {}

/// The spike. If replaying a minimal closure and a causal closure disagree about
/// the change's own lines, the reduction is unsound and layer 1 does not work.
#[test]
#[ignore = "skeleton"]
fn i9_minimal_closure_loses_nothing_against_causal_closure() {}

// --- moves ------------------------------------------------------------------

#[test]
#[ignore = "skeleton"]
fn i10_concurrent_line_moves_leave_exactly_one_copy() {}

#[test]
#[ignore = "skeleton"]
fn i11_concurrent_node_moves_never_cycle_and_all_replicas_skip_the_same_one() {}

// --- liveness ---------------------------------------------------------------

#[test]
#[ignore = "skeleton"]
fn i12_two_replicas_converge_after_a_bidirectional_sync() {}

#[test]
#[ignore = "skeleton"]
fn i13_integrate_is_idempotent_and_order_insensitive() {}
