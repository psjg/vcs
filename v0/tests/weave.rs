//! The live weave against the batch replay (the sum tree has its own file). Skeleton: each test names a law
//! the implementation must hold; bodies come after review.
//!
//! The oracle throughout is `replay`: whatever the weave says about a set of
//! events, materialising the set from scratch must agree.

use proptest::prelude::*;
use v0::sumtree::Summary;
use v0::weave::Metrics;

/// LSP's reading of a text: lines broken by `\r\n`, `\r` or `\n`, columns in
/// UTF-16. Written the obvious way, as the reference for `Metrics`.
fn reference(text: &str) -> (u32, u32) {
    let (mut lines, mut col) = (0u32, 0u32);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\r' => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                lines += 1;
                col = 0;
            }
            '\n' => {
                lines += 1;
                col = 0;
            }
            _ => col += c.len_utf16() as u32,
        }
    }
    (lines, col)
}

fn text() -> impl Strategy<Value = String> {
    proptest::collection::vec(prop_oneof![Just('a'), Just('\r'), Just('\n'), Just('é'), Just('😀')], 0..40)
        .prop_map(|v| v.into_iter().collect())
}

proptest! {
    /// Metrics are a monoid homomorphism: the metrics of a concatenation are
    /// the sum of the parts' -- wherever the cut falls, a `\r\n` torn in two
    /// included -- and they read the text the way LSP does.
    #[test]
    fn metrics_of_a_concatenation_is_the_sum(t in text(), cut in 0usize..41) {
        let cut = t.char_indices().map(|(i, _)| i).chain([t.len()]).nth(cut).unwrap_or(t.len());
        let (a, b) = t.split_at(cut);
        let mut sum = Metrics::of(a);
        sum.add(&Metrics::of(b));
        let whole = Metrics::of(&t);
        prop_assert_eq!(sum, whole);
        prop_assert_eq!((whole.lines, whole.last_utf16), reference(&t));
        let tail = t.rsplit(['\r', '\n']).next().unwrap_or("");
        prop_assert_eq!(whole.last_chars as usize, tail.chars().count());
        prop_assert_eq!(whole.bytes, t.len());
        prop_assert_eq!(whole.chars, t.chars().count());
        prop_assert_eq!(whole.utf16, t.encode_utf16().count());
    }
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
