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
//! **Bounded.** A node holds at most [`MAX`] entries (TigerStyle: every loop
//! and every buffer has a fixed upper bound).
//!
//! The laws, which the property tests hold every operation to:
//!
//! - `summary()` is the fold of `add` over the items' summaries, in order.
//! - `split` then `append` gives back the same items.
//! - every node except the root holds between `MAX / 2` and `MAX` entries, and
//!   every leaf sits at the same depth.

use std::sync::Arc;

/// Most items in a leaf, and most children of an internal node.
pub const MAX: usize = 16;

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

    /// The measure of `s` alone.
    fn of(s: &S) -> Self {
        let mut d = Self::default();
        d.add_summary(s);
        d
    }
}

/// Which item a seek lands on when the target falls exactly on a boundary
/// between two items.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bias {
    /// The item that ends there.
    Left,
    /// The item that starts there.
    Right,
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
#[derive(Clone)]
pub struct SumTree<T: Item>(Arc<Node<T>>);

#[derive(Clone)]
enum Node<T: Item> {
    /// `sums[i]` is `items[i].summary()`, cached: computing it can be real work
    /// (counting UTF-16 units of a fragment's text).
    Leaf { summary: T::Summary, items: Vec<T>, sums: Vec<T::Summary> },
    /// `height` is 1 for a node over leaves; all children share one height.
    Internal { summary: T::Summary, height: u8, children: Vec<SumTree<T>> },
}

impl<T: Item> Default for SumTree<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Item> SumTree<T> {
    /// The empty sequence.
    pub fn new() -> Self {
        todo!()
    }

    /// Build bottom-up in O(n): leaves of [`MAX`] items, then parents over
    /// them, level by level. Loading a document goes through here, not through
    /// `n` pushes.
    pub fn from_items(items: impl IntoIterator<Item = T>) -> Self {
        todo!()
    }

    /// The sum of the whole sequence.
    pub fn summary(&self) -> &T::Summary {
        todo!()
    }

    pub fn is_empty(&self) -> bool {
        todo!()
    }

    /// Every item, in order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        std::iter::empty::<&T>() // TODO: an explicit stack of (node, index); no recursion
    }

    /// The item containing position `target` in dimension `D`.
    ///
    /// An item *contains* `target` when the measure before it is below `target`
    /// and the measure after it reaches it — or, at an exact boundary, as `bias`
    /// says. `None` past the end, or on an empty tree.
    pub fn find<D: Dimension<T::Summary>>(&self, target: &D, bias: Bias) -> Option<Hit<'_, T>> {
        todo!()
    }

    /// Cut into the items before `target` and the rest, deciding a boundary as
    /// [`SumTree::find`] does: the found item starts the right half.
    ///
    /// Item-granular: an item is never cut. A caller that needs to cut inside
    /// one (a fragment of text) finds it, splits the item itself, and
    /// reassembles with [`SumTree::append`] — the item knows how to split, the
    /// tree does not.
    pub fn split<D: Dimension<T::Summary>>(&self, target: &D, bias: Bias) -> (Self, Self) {
        todo!()
    }

    /// Concatenate, in O(log n): the shorter tree is grafted onto the taller
    /// one's edge, and overflowing nodes split on the way back up.
    pub fn append(&mut self, other: Self) {
        todo!()
    }

    /// Add one item at the end.
    pub fn push(&mut self, item: T) {
        todo!()
    }
}
