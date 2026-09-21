//! The sum tree's laws, over random sequences of edits and at several
//! branching factors -- the benchmark will pick one, and every one must hold.

use proptest::prelude::*;
use v0::sumtree::{Bias, Dimension, Item, SumTree, Summary};

/// Items are numbers; the summary is how many, their sum, and the largest.
#[derive(Clone, Debug, PartialEq)]
struct N(u32);

#[derive(Clone, Default, Debug, PartialEq)]
struct S {
    count: usize,
    sum: u64,
    max: u32,
}

impl Summary for S {
    fn add(&mut self, o: &S) {
        self.count += o.count;
        self.sum += o.sum;
        self.max = self.max.max(o.max);
    }
}

impl Item for N {
    type Summary = S;
    fn summary(&self) -> S {
        S { count: 1, sum: self.0 as u64, max: self.0 }
    }
}

/// Position by item count.
#[derive(Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Count(usize);
impl Dimension<S> for Count {
    fn add_summary(&mut self, s: &S) {
        self.0 += s.count;
    }
}

/// Position by running sum: zero-valued items take no room in it, the way
/// tombstones take no room in text coordinates.
#[derive(Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Sum(u64);
impl Dimension<S> for Sum {
    fn add_summary(&mut self, s: &S) {
        self.0 += s.sum;
    }
}

fn fold(items: &[u32]) -> S {
    let mut s = S::default();
    for i in items {
        s.add(&N(*i).summary());
    }
    s
}

fn items<const B: usize>(t: &SumTree<N, B>) -> Vec<u32> {
    t.iter().map(|n| n.0).collect()
}

#[derive(Clone, Debug)]
enum Edit {
    Push(u32),
    Append(Vec<u32>),
    /// Split at this item count and keep the halves in the other order.
    Rotate(usize),
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        (0u32..4).prop_map(Edit::Push),
        proptest::collection::vec(0u32..4, 0..80).prop_map(Edit::Append),
        (0usize..200).prop_map(Edit::Rotate),
    ]
}

/// Apply `edits` to a tree and to a plain Vec side by side; after every edit
/// the tree must hold the Vec's items, sum to their fold, and keep its shape.
fn model_check<const B: usize>(start: Vec<u32>, edits: Vec<Edit>) -> Result<(), TestCaseError> {
    let mut tree: SumTree<N, B> = SumTree::from_items(start.iter().map(|i| N(*i)));
    let mut model = start;
    for e in edits {
        match e {
            Edit::Push(i) => {
                tree.push(N(i));
                model.push(i);
            }
            Edit::Append(v) => {
                tree.append(SumTree::from_items(v.iter().map(|i| N(*i))));
                model.extend(v);
            }
            Edit::Rotate(at) => {
                let at = at.min(model.len());
                let (l, mut r) = tree.split(&Count(at), Bias::Right);
                prop_assert_eq!(items(&l), model[..at].to_vec());
                r.append(l);
                tree = r;
                model.rotate_left(at);
            }
        }
        prop_assert_eq!(items(&tree), model.clone());
        prop_assert_eq!(tree.summary(), &fold(&model));
        prop_assert_eq!(tree.is_empty(), model.is_empty());
        prop_assert_eq!(tree.first().map(|n| n.0), model.first().copied());
        prop_assert_eq!(tree.last().map(|n| n.0), model.last().copied());
        tree.check().map_err(TestCaseError::fail)?;
    }
    Ok(())
}

proptest! {
    /// `summary()` is the fold of the items, and every node holds between
    /// B/2 and B entries at one depth, after any sequence of edits.
    #[test]
    fn summaries_and_shape_hold_under_any_edits(
        start in proptest::collection::vec(0u32..4, 0..300),
        edits in proptest::collection::vec(edit(), 0..20),
    ) {
        model_check::<4>(start.clone(), edits.clone())?;
        model_check::<16>(start.clone(), edits.clone())?;
        model_check::<64>(start, edits)?;
    }

    /// Split anywhere, in either dimension and bias, then append: the same
    /// items. The found item always starts the right half.
    #[test]
    fn split_then_append_is_identity(
        v in proptest::collection::vec(0u32..4, 0..300),
        at in 0u64..400,
        right in any::<bool>(),
    ) {
        let bias = if right { Bias::Right } else { Bias::Left };
        let tree: SumTree<N, 4> = SumTree::from_items(v.iter().map(|i| N(*i)));
        let (mut l, r) = tree.split(&Sum(at), bias);
        let first_right = r.iter().next().map(|n| n.0);
        prop_assert_eq!(tree.find(&Sum(at), bias).map(|h| h.item.0), first_right);
        l.check().map_err(TestCaseError::fail)?;
        r.check().map_err(TestCaseError::fail)?;
        l.append(r);
        prop_assert_eq!(items(&l), v);
    }

    /// `find` agrees with a linear scan, and `before` is the fold of what
    /// precedes the hit.
    #[test]
    fn find_agrees_with_a_scan(
        v in proptest::collection::vec(0u32..4, 0..300),
        at in 0u64..400,
        right in any::<bool>(),
    ) {
        let bias = if right { Bias::Right } else { Bias::Left };
        let tree: SumTree<N, 4> = SumTree::from_items(v.iter().map(|i| N(*i)));
        let mut acc = 0u64;
        let mut expect = None;
        for (i, x) in v.iter().enumerate() {
            let end = acc + *x as u64;
            let hit = if right { end > at } else { end >= at };
            if hit {
                expect = Some(i);
                break;
            }
            acc = end;
        }
        let got = tree.find(&Sum(at), bias);
        prop_assert_eq!(got.as_ref().map(|h| h.item.0), expect.map(|i| v[i]));
        if let (Some(h), Some(i)) = (got, expect) {
            prop_assert_eq!(h.before, fold(&v[..i]));
        }
    }
}

/// A clone is a snapshot: editing the original leaves it untouched.
#[test]
fn a_clone_is_an_independent_snapshot() {
    let mut t: SumTree<N, 4> = SumTree::from_items((0..100).map(N));
    let snap = t.clone();
    t.push(N(7));
    let (l, _) = t.split(&Count(10), Bias::Right);
    t = l;
    assert_eq!(items(&snap), (0..100).collect::<Vec<_>>());
    assert_eq!(items(&t).len(), 10);
}
