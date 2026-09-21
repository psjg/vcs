//! The live weave against the batch replay (the sum tree has its own file). Skeleton: each test names a law
//! the implementation must hold; bodies come after review.
//!
//! The oracle throughout is `replay`: whatever the weave says about a set of
//! events, materialising the set from scratch must agree.

/// Metrics are a monoid: `of(a + b) == of(a).add(of(b))` for any strings,
/// including newlines, CRLF and astral characters (two UTF-16 units).
#[test]
#[ignore = "skeleton"]
fn metrics_of_a_concatenation_is_the_sum() {
    todo!()
}

/// Convergence: folding `Weave::apply` over *any* causal order of a set's
/// events gives the text `replay` materialises for that set.
#[test]
#[ignore = "skeleton"]
fn applying_events_in_any_causal_order_matches_replay() {
    todo!()
}

/// `from_walk` and folding `apply` build the same weave.
#[test]
#[ignore = "skeleton"]
fn bulk_build_equals_incremental_build() {
    todo!()
}

/// Conversions round-trip: `pos_at(offset_of(p))` is `p` for every visible
/// character, and point <-> offset is a bijection on valid positions.
#[test]
#[ignore = "skeleton"]
fn coordinates_round_trip() {
    todo!()
}

/// Live capture agrees with save capture: typing through `insert_op` and
/// `delete_ops` yields the same text as `capture::from_save` on the result.
#[test]
#[ignore = "skeleton"]
fn live_ops_reproduce_what_a_save_would() {
    todo!()
}

/// The `Edit` returned by `apply` transforms the old text into the new one.
#[test]
#[ignore = "skeleton"]
fn reported_edits_replay_on_the_old_text() {
    todo!()
}
