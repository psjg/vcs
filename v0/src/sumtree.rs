//! A persistent B-tree whose nodes carry monoid summaries of their subtrees.
//!
//! The one data structure behind ropes, piece tables and list CRDT indexes: a
//! sequence of items where every internal node knows the *sum* of everything
//! below it. Seeking by any measure derivable from that sum (characters, lines,
//! UTF-16 units, a key) is a walk down one path: O(log n).
//!
//! ```text
//!                 [Σ = a+b+c+d+e+f]
//!                /                  \
//!         [Σ = a+b+c]            [Σ = d+e+f]
//!         /    |    \            /    |    \
//!        a     b     c          d     e     f      <- items, in order
//! ```
//!
//! Design borrowed from Zed's `sum_tree` crate (Apache-2.0): the split into
//! [`Item`], [`Summary`] and [`Dimension`], and seeking in whichever dimension
//! the caller needs. Written fresh and much smaller here: no cursor type, no
//! parallel building, no filtering.
//!
//! **Persistent.** Nodes are shared through [`Arc`]; cloning a tree is O(1) and
//! an edit copies only the path it touches. A snapshot is a clone: checkpoints,
//! "the text as that author had it" (diff3 by author), and an editor's view
//! while a sync lands all cost nothing until they diverge.
//!
//! **Bounded.** A node holds at most `B` entries (TigerStyle: every loop and
//! every buffer has a fixed upper bound). `B` is a const parameter, 16 by
//! default, so a benchmark can sweep it before anyone settles on a number.
//!
//! The laws, which the property tests hold every operation to:
//!
//! - `summary()` is the fold of `add` over the items' summaries, in order.
//! - `split` then `append` gives back the same items.
//! - every node except the root holds between `B / 2` and `B` entries, an
//!   internal root at least two, and every leaf sits at the same depth
//!   ([`SumTree::check`]).

use std::sync::Arc;

/// The default branching factor: most items in a leaf, most children of an
/// internal node. A guess until the benchmark says otherwise; the benchmark
/// sweeps it by building with `V0_SUMTREE_B` set.
pub const DEFAULT_B: usize = match option_env!("V0_SUMTREE_B") {
    Some(s) => parse_usize(s),
    None => 16,
};

/// A decimal number, at compile time: how a build-time knob becomes a const.
pub const fn parse_usize(s: &str) -> usize {
    let b = s.as_bytes();
    let (mut i, mut n) = (0, 0);
    while i < b.len() {
        assert!(b[i].is_ascii_digit(), "not a decimal number");
        n = n * 10 + (b[i] - b'0') as usize;
        i += 1;
    }
    n
}

/// A monoid: `Default` is the identity, [`Summary::add`] is associative.
///
/// Not commutative, and must not be: order is what the tree is for. Text
/// metrics are the model example — `"ab\n" + "c"` ends at line 1, column 1;
/// the other way round it ends at line 1, column 0.
pub trait Summary: Clone + Default + std::fmt::Debug {
    /// `self = self + other`, with `other` coming after.
    fn add(&mut self, other: &Self);
}

/// Something that can be stored in a [`SumTree`].
pub trait Item: Clone {
    type Summary: Summary;
    fn summary(&self) -> Self::Summary;
}

/// A coordinate read off a summary: how far along the sequence you are, in
/// some unit.
///
/// Must be monotone: adding a summary never makes it smaller. That is what
/// lets a seek decide which child to descend into by looking at sums alone.
pub trait Dimension<S: Summary>: Clone + Default + Ord + std::fmt::Debug {
    /// `self = self + (the measure of s)`.
    fn add_summary(&mut self, s: &S);
}

/// Which item a seek lands on when the target falls exactly on a boundary
/// between two items.
///
/// With an item's extent `[before, end)` in the seek's dimension, `Right`
/// takes the first item with `end > target`, `Left` the first with
/// `end >= target`. An item of zero width (a tombstone, measured in text) is
/// therefore skipped by `Right` and can be hit by `Left`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bias {
    /// The item that ends there.
    Left,
    /// The item that starts there.
    Right,
}

impl Bias {
    fn reaches<D: Ord>(self, end: &D, target: &D) -> bool {
        match self {
            Bias::Left => end >= target,
            Bias::Right => end > target,
        }
    }
}

/// Where a seek landed.
#[derive(Debug)]
pub struct Hit<'a, T: Item> {
    /// The summary of every item strictly before `item`. Every dimension can be
    /// read off it, which is how one seek converts between coordinate systems:
    /// seek by line and column, read the character offset off `before`.
    pub before: T::Summary,
    pub item: &'a T,
}

/// A sequence of `T`, indexed by every dimension of `T::Summary`.
pub struct SumTree<T: Item, const B: usize = DEFAULT_B>(Arc<Node<T, B>>);

// Not derived: a derive would demand `T: Clone` on the Arc it only bumps.
impl<T: Item, const B: usize> Clone for SumTree<T, B> {
    fn clone(&self) -> Self {
        SumTree(Arc::clone(&self.0))
    }
}

enum Node<T: Item, const B: usize> {
    /// `sums[i]` is `items[i].summary()`, cached: computing it can be real work
    /// (counting UTF-16 units of a fragment's text).
    Leaf { summary: T::Summary, items: Vec<T>, sums: Vec<T::Summary> },
    /// `height` is 1 for a node over leaves; all children share one height.
    Internal { summary: T::Summary, height: u8, children: Vec<SumTree<T, B>> },
}

impl<T: Item, const B: usize> Default for SumTree<T, B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Item, const B: usize> SumTree<T, B> {
    const _B_IS_SANE: () = assert!(B >= 4, "a branching factor below 4 cannot split evenly");

    /// The empty sequence.
    pub fn new() -> Self {
        let () = Self::_B_IS_SANE;
        Self::leaf(Vec::new(), Vec::new())
    }

    /// Build bottom-up in O(n): leaves of at most `B` items, then parents over
    /// them, level by level. Loading a document goes through here, not through
    /// `n` pushes.
    pub fn from_items(items: impl IntoIterator<Item = T>) -> Self {
        let items: Vec<T> = items.into_iter().collect();
        let mut level: Vec<Self> = even_chunks::<T, B>(items)
            .into_iter()
            .map(|chunk| {
                let sums = chunk.iter().map(Item::summary).collect();
                Self::leaf(chunk, sums)
            })
            .collect();
        let mut height = 0;
        while level.len() > 1 {
            height += 1;
            level = even_chunks::<Self, B>(level).into_iter().map(|kids| Self::internal(height, kids)).collect();
        }
        level.pop().unwrap_or_default()
    }

    /// The sum of the whole sequence.
    pub fn summary(&self) -> &T::Summary {
        match &*self.0 {
            Node::Leaf { summary, .. } | Node::Internal { summary, .. } => summary,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(&*self.0, Node::Leaf { items, .. } if items.is_empty())
    }

    /// The first item, down the left edge: O(log n).
    pub fn first(&self) -> Option<&T> {
        let mut node = self;
        loop {
            match &*node.0 {
                Node::Leaf { items, .. } => return items.first(),
                Node::Internal { children, .. } => node = children.first()?,
            }
        }
    }

    /// The last item, down the right edge: O(log n).
    pub fn last(&self) -> Option<&T> {
        let mut node = self;
        loop {
            match &*node.0 {
                Node::Leaf { items, .. } => return items.last(),
                Node::Internal { children, .. } => node = children.last()?,
            }
        }
    }

    /// Every item, in order.
    pub fn iter(&self) -> Iter<'_, T, B> {
        Iter { stack: vec![(self, 0)] }
    }

    /// The item containing position `target` in dimension `D`, deciding
    /// boundaries as [`Bias`] says. `None` past the end, or on an empty tree.
    pub fn find<D: Dimension<T::Summary>>(&self, target: &D, bias: Bias) -> Option<Hit<'_, T>> {
        let mut before = T::Summary::default();
        let mut at = D::default();
        let mut node = self;
        loop {
            match &*node.0 {
                Node::Internal { children, .. } => {
                    let i = children.iter().position(|c| {
                        let mut end = at.clone();
                        end.add_summary(c.summary());
                        let hit = bias.reaches(&end, target);
                        if !hit {
                            before.add(c.summary());
                            at = end;
                        }
                        hit
                    })?;
                    node = &children[i];
                }
                Node::Leaf { items, sums, .. } => {
                    let i = sums.iter().position(|s| {
                        let mut end = at.clone();
                        end.add_summary(s);
                        let hit = bias.reaches(&end, target);
                        if !hit {
                            before.add(s);
                            at = end;
                        }
                        hit
                    })?;
                    return Some(Hit { before, item: &items[i] });
                }
            }
        }
    }

    /// Cut into the items before `target` and the rest, deciding a boundary as
    /// [`SumTree::find`] does: the found item starts the right half. Past the
    /// end, everything goes left.
    ///
    /// Item-granular: an item is never cut. A caller that needs to cut inside
    /// one (a fragment of text) finds it, splits the item itself, and
    /// reassembles with [`SumTree::append`] — the item knows how to split, the
    /// tree does not.
    pub fn split<D: Dimension<T::Summary>>(&self, target: &D, bias: Bias) -> (Self, Self) {
        let mut at = D::default();
        self.split_from(&mut at, target, bias)
    }

    fn split_from<D: Dimension<T::Summary>>(&self, at: &mut D, target: &D, bias: Bias) -> (Self, Self) {
        match &*self.0 {
            Node::Leaf { items, sums, .. } => {
                let i = sums
                    .iter()
                    .position(|s| {
                        let mut end = at.clone();
                        end.add_summary(s);
                        let hit = bias.reaches(&end, target);
                        if !hit {
                            *at = end;
                        }
                        hit
                    })
                    .unwrap_or(items.len());
                let left = Self::leaf(items[..i].to_vec(), sums[..i].to_vec());
                let right = Self::leaf(items[i..].to_vec(), sums[i..].to_vec());
                (left, right)
            }
            Node::Internal { children, height, .. } => {
                let i = children.iter().position(|c| {
                    let mut end = at.clone();
                    end.add_summary(c.summary());
                    let hit = bias.reaches(&end, target);
                    if !hit {
                        *at = end;
                    }
                    hit
                });
                let Some(i) = i else { return (self.clone(), Self::new()) };
                // Whole children stay whole; only the one containing the
                // target is cut, recursively. Reassembly goes through append,
                // which restores every node-size bound.
                let (mid_l, mid_r) = children[i].split_from(at, target, bias);
                let mut left = Self::root_over(*height, &children[..i]);
                left.append(mid_l);
                let mut right = mid_r;
                right.append(Self::root_over(*height, &children[i + 1..]));
                (left, right)
            }
        }
    }

    /// Concatenate, in O(log n): the shorter tree is grafted onto the taller
    /// one's edge at its own height, and overflowing nodes split on the way
    /// back up.
    pub fn append(&mut self, other: Self) {
        if other.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = other;
            return;
        }
        let (h, g) = (self.height(), other.height());
        let (node, extra) = if h >= g { graft(self, other, g, true) } else { graft(&other, self.clone(), h, false) };
        *self = match extra {
            None => node,
            Some(sibling) => Self::internal(node.height() + 1, vec![node, sibling]),
        };
    }

    /// Add one item at the end.
    pub fn push(&mut self, item: T) {
        self.append(Self::from_items([item]));
    }

    /// Check the shape laws: node sizes, one leaf depth, and that every cached
    /// summary is the fold of what it covers (by the `Debug` rendering, since
    /// summaries need not be `PartialEq`). For tests; O(n).
    pub fn check(&self) -> Result<(), String> {
        self.check_node(true).map(|_| ())
    }

    fn check_node(&self, root: bool) -> Result<u8, String> {
        let (n, fold, height) = match &*self.0 {
            Node::Leaf { items, sums, .. } => {
                if items.len() != sums.len() {
                    return Err("leaf: items and sums differ in length".into());
                }
                (items.len(), fold(sums.iter()), 0)
            }
            Node::Internal { children, height, .. } => {
                for c in children {
                    let h = c.check_node(false)?;
                    if h + 1 != *height {
                        return Err(format!("child at height {h} under a node at height {height}"));
                    }
                }
                if root && children.len() < 2 {
                    return Err("internal root with fewer than two children".into());
                }
                (children.len(), fold(children.iter().map(|c| c.summary())), *height)
            }
        };
        if n > B || (!root && n < B / 2) {
            return Err(format!("node of {n} entries at height {height}, bounds {}..={B}", B / 2));
        }
        if format!("{fold:?}") != format!("{:?}", self.summary()) {
            return Err(format!("cached summary {:?} is not the fold {fold:?}", self.summary()));
        }
        Ok(height)
    }

    /// A tree over some whole children of one node, in O(k): no children is
    /// empty, one is that child, more make a root -- which may hold as few as
    /// two, so the bounds hold without any rebalancing.
    fn root_over(height: u8, children: &[Self]) -> Self {
        match children {
            [] => Self::new(),
            [one] => one.clone(),
            many => Self::internal(height, many.to_vec()),
        }
    }

    fn height(&self) -> u8 {
        match &*self.0 {
            Node::Leaf { .. } => 0,
            Node::Internal { height, .. } => *height,
        }
    }

    fn leaf(items: Vec<T>, sums: Vec<T::Summary>) -> Self {
        let summary = fold(sums.iter());
        SumTree(Arc::new(Node::Leaf { summary, items, sums }))
    }

    fn internal(height: u8, children: Vec<Self>) -> Self {
        let summary = fold(children.iter().map(|c| c.summary()));
        SumTree(Arc::new(Node::Internal { summary, height, children }))
    }

    /// Two nodes of one height as one, or two if they do not fit in one.
    fn merge(a: &Self, b: &Self) -> (Self, Option<Self>) {
        match (&*a.0, &*b.0) {
            (Node::Leaf { items: ia, sums: sa, .. }, Node::Leaf { items: ib, sums: sb, .. }) => {
                let items: Vec<T> = ia.iter().chain(ib).cloned().collect();
                let sums: Vec<T::Summary> = sa.iter().chain(sb).cloned().collect();
                if items.len() <= B {
                    return (Self::leaf(items, sums), None);
                }
                let mid = items.len() / 2;
                let right = Self::leaf(items[mid..].to_vec(), sums[mid..].to_vec());
                (Self::leaf(items[..mid].to_vec(), sums[..mid].to_vec()), Some(right))
            }
            (Node::Internal { children: ca, height, .. }, Node::Internal { children: cb, .. }) => {
                Self::split_children(*height, ca.iter().chain(cb).cloned().collect())
            }
            _ => unreachable!("merge is only called on nodes of one height"),
        }
    }

    /// An internal node over `children`, split in two halves if they overflow.
    fn split_children(height: u8, mut children: Vec<Self>) -> (Self, Option<Self>) {
        if children.len() <= B {
            return (Self::internal(height, children), None);
        }
        let right = children.split_off(children.len() / 2);
        (Self::internal(height, children), Some(Self::internal(height, right)))
    }
}

/// Put `other` (height `g`) at the right edge of `host` (height >= g) when
/// `right`, else put `other` at the left edge of `host`. Returns the new host
/// and, if it overflowed, the sibling that goes to its right.
///
/// At equal height the two nodes merge. Above it, the graft recurses into the
/// edge child, so the small tree lands at its own depth and every leaf stays
/// at one depth.
#[allow(clippy::type_complexity)]
fn graft<T: Item, const B: usize>(
    host: &SumTree<T, B>,
    other: SumTree<T, B>,
    g: u8,
    right: bool,
) -> (SumTree<T, B>, Option<SumTree<T, B>>) {
    if host.height() == g {
        return if right { SumTree::merge(host, &other) } else { SumTree::merge(&other, host) };
    }
    let Node::Internal { children, height, .. } = &*host.0 else { unreachable!("height > g > = 0") };
    let mut kids = children.clone();
    let edge = if right { kids.len() - 1 } else { 0 };
    let (node, extra) = graft(&kids[edge], other, g, right);
    kids[edge] = node;
    if let Some(sib) = extra {
        kids.insert(edge + 1, sib);
    }
    SumTree::split_children(*height, kids)
}

/// Cut `v` into the fewest chunks of at most `B`, as even as possible: with
/// more than one chunk each holds more than `B / 2`, which is what keeps a
/// freshly built tree inside its bounds.
fn even_chunks<X, const B: usize>(v: Vec<X>) -> Vec<Vec<X>> {
    if v.is_empty() {
        return Vec::new();
    }
    let k = v.len().div_ceil(B);
    let (base, extra) = (v.len() / k, v.len() % k);
    let mut it = v.into_iter();
    (0..k).map(|i| it.by_ref().take(base + usize::from(i < extra)).collect()).collect()
}

fn fold<'a, S: Summary + 'a>(sums: impl Iterator<Item = &'a S>) -> S {
    let mut acc = S::default();
    for s in sums {
        acc.add(s);
    }
    acc
}

/// In-order iteration with an explicit stack: no recursion, so depth is never
/// a stack-overflow question.
pub struct Iter<'a, T: Item, const B: usize> {
    stack: Vec<(&'a SumTree<T, B>, usize)>,
}

impl<'a, T: Item, const B: usize> Iterator for Iter<'a, T, B> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        loop {
            let (node, i) = self.stack.last_mut()?;
            let node: &'a SumTree<T, B> = node;
            match &*node.0 {
                Node::Leaf { items, .. } => {
                    if let Some(item) = items.get(*i) {
                        *i += 1;
                        return Some(item);
                    }
                    self.stack.pop();
                }
                Node::Internal { children, .. } => match children.get(*i) {
                    Some(c) => {
                        *i += 1;
                        self.stack.push((c, 0));
                    }
                    None => {
                        self.stack.pop();
                    }
                },
            }
        }
    }
}

/// The checker is the verifier every property test leans on, so it gets
/// tested itself: each law, broken by hand, must be reported.
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug)]
    struct U;
    impl Summary for usize {
        fn add(&mut self, o: &usize) {
            *self += o;
        }
    }
    impl Item for U {
        type Summary = usize;
        fn summary(&self) -> usize {
            1
        }
    }
    type T4 = SumTree<U, 4>;

    fn leaf(n: usize) -> T4 {
        T4::leaf(vec![U; n], vec![1; n])
    }

    #[test]
    fn a_valid_tree_passes() {
        assert_eq!(T4::from_items(vec![U; 50]).check(), Ok(()));
        assert!(T4::new().is_empty() && T4::from_items([]).is_empty());
        assert!(!T4::from_items([U]).is_empty());
    }

    #[test]
    fn each_broken_law_is_reported() {
        let overfull = leaf(5);
        assert!(overfull.check().unwrap_err().contains("entries"));

        let underfull = T4::internal(1, vec![leaf(2), leaf(1)]);
        assert!(underfull.check().unwrap_err().contains("entries"), "a non-root leaf of 1 < 4/2");

        let lonely = T4::internal(1, vec![leaf(3)]);
        assert!(lonely.check().unwrap_err().contains("fewer than two"));

        let ragged = T4::internal(2, vec![T4::internal(1, vec![leaf(2), leaf(2)]), leaf(2)]);
        assert!(ragged.check().unwrap_err().contains("height"));

        let lying = SumTree::<U, 4>(Arc::new(Node::Leaf { summary: 9, items: vec![U; 2], sums: vec![1; 2] }));
        assert!(lying.check().unwrap_err().contains("fold"));

        let torn = SumTree::<U, 4>(Arc::new(Node::Leaf { summary: 1, items: vec![U; 2], sums: vec![1] }));
        assert!(torn.check().unwrap_err().contains("length"));
    }
}
