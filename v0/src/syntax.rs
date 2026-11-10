//! The structure layer: a syntax tree that follows a weave.
//!
//! The weave says *where* and remembers it: characters with anchors that
//! survive every edit. The tree says *what*: nodes, recomputed after every
//! edit, with no identity of their own. This module is the bridge, in both
//! directions, and each crossing is O(log n):
//!
//! - node -> its span -> the anchors of its first and last character
//!   ([`Weave::pos_at`]): a [`NodeRef`], the only form a node takes outside
//!   this module and the only one an op may name;
//! - anchors -> offsets ([`Weave::offset_of`]) -> the node on that span
//!   (`descendant_for_byte_range`).
//!
//! The weave is also the parser's input: tree-sitter reads text through a
//! callback that asks for the chunk at a byte ([`Weave::chunk_at`]).
//!
//! **Structural ops refer to anchors, never to nodes.** A [`Op::Rename`]
//! names the anchor of the declared identifier; applying it finds the node,
//! its references through a scope walk of the tree ([`Scopes`]), their
//! anchors, and from those the plain character ops that do the work. The character CRDT
//! stays the ground truth of the text; this layer is how intents are
//! computed, and where they are re-computed when a merge brings a
//! reference the intent had not seen (see [`reapply`]).
//!
//! Structure is a property of the document type, not of the store. A
//! document without a grammar has none of this and loses nothing else;
//! lines and paragraphs as nodes are the next rung down (TECHDEBT), not a
//! trait to design ahead of them.
//!
//! The oracle: after any sequence of edits, the tree followed incrementally
//! equals a fresh parse of the text (`tests/syntax.rs`).

use crate::event::{Event, EventLog};
use crate::op::{Anchor, EventId, Op, Pos, ReplicaId, Side};
use crate::sumtree::Bias;
use crate::weave::{Chars, Edit, Metrics, Weave};
use std::ops::Range;
use tree_sitter::{InputEdit, Language, Node, Parser, Point, Tree};

/// Which grammar a document is read with. Rust first; Markdown second, and
/// only its block grammar: the inline one is a second parse injected into
/// each paragraph, and nothing here needs it yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grammar {
    Rust,
    Markdown,
}

impl Grammar {
    /// The grammar for a file name, by extension. `None`: no structure.
    pub fn for_name(name: &str) -> Option<Grammar> {
        match name.rsplit_once('.')?.1 {
            "rs" => Some(Grammar::Rust),
            "md" => Some(Grammar::Markdown),
            _ => None,
        }
    }

    fn language(self) -> Language {
        match self {
            Grammar::Rust => tree_sitter_rust::LANGUAGE.into(),
            Grammar::Markdown => tree_sitter_md::LANGUAGE.into(),
        }
    }
}

/// A syntax node named by anchors: the first and last character of its span.
/// Stable across edits, unlike a `tree_sitter::Node`, and re-found from the
/// weave in O(log n). The node it names may have grown, shrunk or ceased to
/// parse as that kind; [`Syntax::resolve`] says.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct NodeRef {
    pub first: Pos,
    pub last: Pos,
}

/// A place in the text as tree-sitter wants it: row, and byte column.
///
/// Rows here break the way [`Metrics`] breaks lines, at a lone `\r` too,
/// where tree-sitter itself counts only `\n`. Nothing in this module reads
/// a point back -- every crossing is by byte -- so the difference stays
/// inside tree-sitter's position reports (TECHDEBT).
fn point(m: &Metrics) -> Point {
    Point { row: m.lines as usize, column: m.last_bytes as usize }
}

/// One document's tree, kept in step with its weave.
pub struct Syntax {
    grammar: Grammar,
    parser: Parser,
    tree: Tree,
}

impl Syntax {
    /// Parse a weave's whole text. O(n); the once-per-open cost.
    pub fn of(weave: &Weave, grammar: Grammar) -> Syntax {
        let mut parser = Parser::new();
        parser.set_language(&grammar.language()).expect("a grammar built for this tree-sitter");
        let tree = Self::parse(&mut parser, weave, None);
        Syntax { grammar, parser, tree }
    }

    /// The weave is the parser's input: it asks for the text at a byte and
    /// gets the rest of that fragment.
    fn parse(parser: &mut Parser, weave: &Weave, old: Option<&Tree>) -> Tree {
        let mut read = |byte: usize, _: Point| weave.chunk_at(byte);
        parser.parse_with_options(&mut read, old, None).expect("no timeout, no cancellation, a language set")
    }

    /// Follow the edits one [`Weave::apply`] reported, from the weave as it
    /// was (`before`, an O(1) clone taken first) to the weave as it is.
    ///
    /// The edits are sequential -- each in the coordinates the one before
    /// left -- and only the two end states exist as weaves, so they are
    /// folded into the one stretch that covers them all ([`dirty`]), and
    /// that becomes a single tree-sitter `InputEdit`: bytes and points at
    /// its start, old end and new end, read off [`Weave::metrics_at`] in
    /// the two weaves. Then one incremental reparse over `after`. Returns
    /// the nodes whose text or shape changed, as refs.
    pub fn follow(&mut self, before: &Weave, after: &Weave, edits: &[Edit]) -> Vec<NodeRef> {
        let Some(span) = dirty(edits) else { return Vec::new() };
        let grown = after.metrics().chars as isize - before.metrics().chars as isize;
        let old_end = (span.end as isize - grown) as usize;
        let (start, old, new) =
            (after.metrics_at(Chars(span.start)), before.metrics_at(Chars(old_end)), after.metrics_at(Chars(span.end)));
        self.tree.edit(&InputEdit {
            start_byte: start.bytes,
            old_end_byte: old.bytes,
            new_end_byte: new.bytes,
            start_position: point(&start),
            old_end_position: point(&old),
            new_end_position: point(&new),
        });
        let tree = Self::parse(&mut self.parser, after, Some(&self.tree));
        let mut changed: Vec<Range<usize>> = vec![start.bytes..new.bytes];
        changed.extend(self.tree.changed_ranges(&tree).map(|r| r.start_byte..r.end_byte));
        self.tree = tree;
        let mut refs: Vec<NodeRef> = changed
            .into_iter()
            .filter_map(|r| self.node_at(after, after.chars_at_byte(r.start)..after.chars_at_byte(r.end)))
            .collect();
        refs.sort();
        refs.dedup();
        refs
    }

    /// The smallest named node spanning `range`, as a ref. `None` for a
    /// node with no characters: there is nothing to anchor it to.
    pub fn node_at(&self, weave: &Weave, range: Range<Chars>) -> Option<NodeRef> {
        let (a, b) = (weave.metrics_at(range.start).bytes, weave.metrics_at(range.end).bytes);
        let node = self.tree.root_node().named_descendant_for_byte_range(a, b)?;
        self.ref_of(weave, node)
    }

    fn ref_of(&self, weave: &Weave, node: Node) -> Option<NodeRef> {
        let span = chars(weave, &node);
        if span.is_empty() {
            return None;
        }
        Some(NodeRef { first: weave.pos_at(span.start, Bias::Right)?, last: weave.pos_at(span.end, Bias::Left)? })
    }

    /// The node a ref names now, if its span still parses as one node: the
    /// smallest named node covering exactly the characters from its first
    /// anchor to its last. A deleted anchor still has a place -- where it
    /// would be -- so a node that lost an end is looked for on what is
    /// left of it.
    pub fn resolve<'a>(&'a self, weave: &Weave, node: NodeRef) -> Option<Node<'a>> {
        let (first, _) = weave.offset_of(node.first)?;
        let (last, visible) = weave.offset_of(node.last)?;
        let end = Chars(last.0 + usize::from(visible));
        let (a, b) = (weave.metrics_at(first).bytes, weave.metrics_at(end).bytes);
        let found = self.tree.root_node().named_descendant_for_byte_range(a, b)?;
        (a < b && found.byte_range() == (a..b)).then_some(found)
    }

    /// The kind of the node a ref names now (`function_item`, ...).
    pub fn kind(&self, weave: &Weave, node: NodeRef) -> Option<&str> {
        self.resolve(weave, node).map(|n| n.kind())
    }

    /// Every reference to the identifier declared at `decl`, the declaration
    /// included, as character ranges in document order: a scope walk of
    /// this document's tree ([`Scopes`]). Rust only, and empty when `decl`
    /// is not the first character of a declared name; cross-file binding is
    /// a language server's job and out of scope here.
    pub fn references(&self, weave: &Weave, decl: Pos) -> Vec<Range<Chars>> {
        if self.grammar != Grammar::Rust {
            return Vec::new();
        }
        let Some((Chars(at), true)) = weave.offset_of(decl) else { return Vec::new() };
        let text = weave.text();
        let at = weave.metrics_at(Chars(at)).bytes;
        let Some(name) = self.tree.root_node().named_descendant_for_byte_range(at, at + 1) else { return Vec::new() };
        if name.start_byte() != at || !matches!(name.kind(), "identifier" | "type_identifier") {
            return Vec::new();
        }
        let scopes = Scopes::of(&self.tree, &text, &text[name.byte_range()], name.kind());
        let Some(d) = scopes.decls.iter().position(|d| d.name == name.byte_range()) else { return Vec::new() };
        scopes.uses.iter().filter(|u| scopes.binder(u) == Some(d)).map(|u| char_range(weave, &u.at)).collect()
    }

    /// The declaration of the name at `at`, whether `at` is in the
    /// declaration or in a use of it: what "rename this" under a cursor
    /// means. `None` where [`Syntax::references`] would find nothing.
    pub fn declaration_at(&self, weave: &Weave, at: Chars) -> Option<Pos> {
        if self.grammar != Grammar::Rust {
            return None;
        }
        let at = weave.metrics_at(at).bytes;
        let text = weave.text();
        let name = self.tree.root_node().named_descendant_for_byte_range(at, (at + 1).min(text.len()))?;
        let scopes = Scopes::of(&self.tree, &text, text.get(name.byte_range())?, name.kind());
        let decl = match scopes.decls.iter().find(|d| d.name == name.byte_range()) {
            Some(d) => d,
            None => &scopes.decls[scopes.binder(scopes.uses.iter().find(|u| u.at == name.byte_range())?)?],
        };
        weave.pos_at(weave.chars_at_byte(decl.name.start), Bias::Right)
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }
}

/// A node's span in characters.
fn chars(weave: &Weave, node: &Node) -> Range<Chars> {
    char_range(weave, &node.byte_range())
}

fn char_range(weave: &Weave, bytes: &Range<usize>) -> Range<Chars> {
    weave.chars_at_byte(bytes.start)..weave.chars_at_byte(bytes.end)
}

/// The one stretch of the final text that covers every edit of a sequence,
/// each edit being in the coordinates the one before left. `None`: no edits.
pub fn dirty(edits: &[Edit]) -> Option<Range<usize>> {
    let mut span: Option<Range<usize>> = None;
    for e in edits {
        let new = e.text.chars().count();
        span = Some(match span {
            None => e.old.start..e.old.start + new,
            // What the stretch covered moves with the text after this edit.
            Some(s) => s.start.min(e.old.start)..s.end.max(e.old.end) - e.old.len() + new,
        });
    }
    span
}

// --- scopes -------------------------------------------------------------------

/// One declaration of a name: where the name stands, and the stretch of the
/// document in which a use of it means this declaration.
struct Decl {
    name: Range<usize>,
    region: Range<usize>,
    /// A binding, not an item: it does not reach into a function nested in
    /// its region, which can see the items around it and none of the locals.
    local: bool,
}

/// One use of the name, and the innermost function that holds it.
struct Use {
    at: Range<usize>,
    function: Range<usize>,
}

/// Every declaration and every use of one name in a Rust document, in bytes.
///
/// tree-sitter-rust ships no scope query (`locals.scm`), so this is the
/// walk such a query would drive, written out. It is name resolution for
/// one file by syntax alone, and it is an approximation with known edges:
///
/// - an item (`fn`, `const`, `static`, `struct`, `mod`, ...) is in scope in
///   the whole block, module or file that holds it;
/// - a `let` binds from the end of its statement to the end of its block;
///   parameters bind in their function or closure; `for`, `match` arm and
///   `if let`/`while let` patterns bind in the body they guard;
/// - a type or const parameter binds in the item that declares it;
/// - a use means the innermost declaration whose region holds it, the
///   later one where regions nest equally (shadowing); a binding does not
///   reach into a nested `fn`;
/// - **not resolved:** paths (`a::foo`, `a::Foo` are never uses of a local
///   name), names brought in by `use`, struct-literal shorthand, a tuple
///   struct's constructor when the declaration is the type. An associated
///   item (in an `impl` or `trait`) is reached only by path or method call,
///   so it declares nothing here and cannot be renamed. Identifiers inside
///   a macro's tokens are taken as uses: they are, far more often than not.
struct Scopes {
    decls: Vec<Decl>,
    uses: Vec<Use>,
}

impl Scopes {
    fn of(tree: &Tree, text: &str, name: &str, kind: &str) -> Scopes {
        let named = |n: &Node, kind: &str| n.kind() == kind && &text[n.byte_range()] == name;
        let mut out = Scopes { decls: Vec::new(), uses: Vec::new() };
        let mut cursor = tree.walk();
        // Preorder, without recursion: down, else across, else up and across.
        'walk: loop {
            let n = cursor.node();
            out.visit(n, kind, &named);
            if cursor.goto_first_child() {
                continue;
            }
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    break 'walk;
                }
            }
        }
        out
    }

    fn visit(&mut self, n: Node, kind: &str, named: &dyn Fn(&Node, &str) -> bool) {
        if named(&n, kind) {
            let parent = n.parent();
            let field_of = |field: &str| parent.and_then(|p| p.child_by_field_name(field)).is_some_and(|c| c == n);
            let parent_kind = parent.map_or("", |p| p.kind());
            // An item's name is in scope wherever the item's container
            // reaches -- unless that container is an impl or a trait, whose
            // items are not in scope by their bare name at all: such a name
            // declares nothing here, and is no use of anything either.
            let item = field_of("name") && (parent_kind.ends_with("_item") || parent_kind == "macro_definition");
            let holder = enclosing(parent.unwrap_or(n), &["block", "declaration_list", "source_file"]);
            let associated = item && holder.parent().is_some_and(|p| matches!(p.kind(), "impl_item" | "trait_item"));
            let not_a_use = associated
                || (matches!(parent_kind, "scoped_identifier" | "scoped_type_identifier") && field_of("name"))
                || (parent_kind == "macro_invocation" && field_of("macro"));
            if !not_a_use {
                let function = enclosing(n, &["function_item"]);
                self.uses.push(Use { at: n.byte_range(), function: function.byte_range() });
            }
            if item && !associated {
                self.decls.push(Decl { name: n.byte_range(), region: holder.byte_range(), local: false });
            }
            // A generic parameter: bound in the item it is declared on.
            if field_of("name") && matches!(parent_kind, "type_parameter" | "const_parameter") {
                let item = enclosing(n, &["type_parameters"]).parent().map_or(n.byte_range(), |i| i.byte_range());
                self.decls.push(Decl { name: n.byte_range(), region: item, local: false });
            }
        }
        // A binder: every name its pattern introduces.
        let (pattern, region) = match n.kind() {
            "let_declaration" => {
                let end = n.parent().map_or(n.end_byte(), |b| b.end_byte());
                (n.child_by_field_name("pattern"), n.end_byte()..end)
            }
            "parameter" => {
                let f = enclosing(n, &["function_item", "closure_expression", "function_signature_item"]);
                (n.child_by_field_name("pattern"), f.byte_range())
            }
            "closure_parameters" => (Some(n), n.parent().map_or(n.byte_range(), |c| c.byte_range())),
            "for_expression" => {
                (n.child_by_field_name("pattern"), n.child_by_field_name("body").map_or(n.byte_range(), |b| b.byte_range()))
            }
            "match_arm" => (n.child_by_field_name("pattern"), n.byte_range()),
            "let_condition" => {
                // From the end of the condition: the rest of a let chain
                // (`if let Some(x) = a && x > 0`) sees the binding too.
                let guarded = enclosing(n, &["if_expression", "while_expression"]);
                let body = guarded.child_by_field_name("consequence").or_else(|| guarded.child_by_field_name("body"));
                (n.child_by_field_name("pattern"), n.end_byte()..body.map_or(guarded.end_byte(), |b| b.end_byte()))
            }
            _ => (None, 0..0),
        };
        if let Some(pattern) = pattern {
            for name in bound_names(pattern, named) {
                self.decls.push(Decl { name, region: region.clone(), local: true });
            }
        }
    }

    /// Which declaration a use means: itself if it is a declaration's name,
    /// else the innermost region holding it, the later declaration on a tie.
    /// A binding counts only for uses in its own function.
    fn binder(&self, u: &Use) -> Option<usize> {
        if let Some(own) = self.decls.iter().position(|d| d.name == u.at) {
            return Some(own);
        }
        let within = |outer: &Range<usize>, inner: &Range<usize>| outer.start <= inner.start && inner.end <= outer.end;
        (0..self.decls.len())
            .filter(|i| within(&self.decls[*i].region, &u.at))
            .filter(|i| !self.decls[*i].local || within(&u.function, &self.decls[*i].name))
            .min_by_key(|i| (self.decls[*i].region.len(), std::cmp::Reverse(self.decls[*i].name.start)))
    }
}

/// The nearest ancestor of one of these kinds, or the root.
fn enclosing<'a>(mut n: Node<'a>, kinds: &[&str]) -> Node<'a> {
    while let Some(p) = n.parent() {
        n = p;
        if kinds.contains(&n.kind()) {
            break;
        }
    }
    n
}

/// The names a pattern binds, among those `named` picks: every such
/// identifier under it, except the constructor of a tuple or struct pattern
/// and anything in a path -- those name what is matched against.
///
/// `named` is told the kind to compare by, since a struct pattern's
/// shorthand (`S { foo }`) binds `foo` under a kind of its own.
fn bound_names(pattern: Node, named: &dyn Fn(&Node, &str) -> bool) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut todo = vec![pattern];
    while let Some(n) = todo.pop() {
        if matches!(n.kind(), "scoped_identifier" | "scoped_type_identifier" | "closure_expression" | "block") {
            continue;
        }
        let constructor = matches!(n.kind(), "tuple_struct_pattern" | "struct_pattern")
            .then(|| n.child_by_field_name("type"))
            .flatten();
        let mut c = n.walk();
        todo.extend(n.children(&mut c).filter(|k| Some(*k) != constructor));
        if named(&n, "identifier") || named(&n, "shorthand_field_identifier") {
            out.push(n.byte_range());
        }
    }
    out
}

// --- intents ----------------------------------------------------------------

/// Replace the characters `range` with `to`, as ops whose content depends
/// on nothing but the characters replaced: the deletes name them, and the
/// insert hangs to the left of the first of them (Fugue: a left child reads
/// just before its parent). Two replicas replacing the same characters
/// therefore compute the same ops, whatever else is in their worlds.
fn replace(weave: &Weave, range: Range<Chars>, to: &str) -> Vec<Op> {
    let Some(first) = weave.pos_at(range.start, Bias::Right) else { return Vec::new() };
    let mut ops = vec![Op::Insert { parent: Anchor::At(first), side: Side::Left, text: to.to_owned() }];
    ops.extend(weave.delete_ops(range));
    ops
}

/// Expand a rename at record time: for every reference of the identifier
/// declared at `decl`, an insert of `to` and a delete of its characters. The
/// intent itself comes first, as [`Op::Rename`] with the name as it read;
/// the rest are its effect, ordinary events in the weave. Empty when `decl`
/// declares nothing.
pub fn rename(weave: &Weave, syntax: &Syntax, decl: Pos, to: &str) -> Vec<Op> {
    let refs = syntax.references(weave, decl);
    let Some(first) = refs.first() else { return Vec::new() };
    // Every reference reads the same; the old name is any one of them.
    let from: String = weave.text().chars().skip(first.start.0).take(first.end.0 - first.start.0).collect();
    let mut ops = vec![Op::Rename { decl, from, to: to.to_owned() }];
    ops.extend(refs.into_iter().flat_map(|r| replace(weave, r, to)));
    ops
}

/// The id of one op of a rename's effect on a reference it had not seen:
/// the same on every replica that computes the same op, so two merges of
/// the same histories mint the same event and it dedups, rather than
/// renaming twice. The op's content is in the id, so two replicas that
/// somehow derive different ops can never collide on one id. Lamport-valid:
/// later than the rename and than everything the op refers to.
pub fn derived_id(rename: EventId, op: &Op) -> EventId {
    let seq = op.refs().iter().map(|r| r.seq).fold(rename.seq, u32::max) + 1;
    let mut h = blake3::Hasher::new();
    h.update(&rename.seq.to_le_bytes());
    h.update(&rename.replica.0.to_le_bytes());
    h.update(&serde_json::to_vec(op).expect("an op is plain data"));
    let replica = u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().expect("eight bytes"));
    EventId { seq, replica: ReplicaId(replica) }
}

/// At merge: references of the renamed identifier that still read the old
/// name -- inserted concurrently with the rename -- get its effect, as
/// events with [`derived_id`]s. What makes a rename an intent rather than a
/// batch of edits: the acceptance test is Alice renaming `foo` to `bar`
/// while Bob adds a call to `foo`, and Bob's call reading `bar` after the
/// merge.
///
/// A stale reference is a use of the old name, in the scope of the renamed
/// declaration, that no declaration of the old name claims (someone who
/// meanwhile declared their own `foo` keeps it), typed by someone who had
/// not seen the rename. `weave` and `syntax` are the merged document.
/// Returns nothing when `rename` is not a rename in `log`, or its
/// declaration no longer reads the new name. The events are for the caller
/// to integrate and apply; doing so twice changes nothing.
pub fn reapply(weave: &Weave, syntax: &Syntax, log: &EventLog, rename: EventId) -> Vec<Event> {
    let Some(Op::Rename { decl, from, to }) = log.events().get(&rename).map(|e| &e.op) else { return Vec::new() };
    // The declaration now: the new name stands just before the old, deleted
    // one ([`replace`]), so it ends where `decl` would be.
    let Some((Chars(end), _)) = weave.offset_of(*decl) else { return Vec::new() };
    let end = weave.metrics_at(Chars(end)).bytes;
    let text = weave.text();
    let found = syntax.tree.root_node().named_descendant_for_byte_range(end.saturating_sub(1), end);
    let Some(name) = found.filter(|n| n.end_byte() == end && &text[n.byte_range()] == to.as_str()) else {
        return Vec::new();
    };
    let renamed = Scopes::of(&syntax.tree, &text, to, name.kind());
    let Some(region) = renamed.decls.iter().find(|d| d.name == name.byte_range()).map(|d| d.region.clone()) else {
        return Vec::new();
    };

    let stale = Scopes::of(&syntax.tree, &text, from, name.kind());
    let mut out = Vec::new();
    for u in stale.uses.iter().filter(|u| region.start <= u.at.start && u.at.end <= region.end) {
        let span = char_range(weave, &u.at);
        // Typed without having seen the rename -- every character of it. One
        // typed after seeing it means someone wrote the old name on purpose.
        let typed: std::collections::BTreeSet<EventId> =
            (span.start.0..span.end.0).filter_map(|k| weave.pos_at(Chars(k), Bias::Right)).map(|p| p.event).collect();
        if stale.binder(u).is_some() || typed.iter().any(|e| log.causal_closure(*e).contains(&rename)) {
            continue;
        }
        for op in replace(weave, span, to) {
            let mut parents = op.refs();
            parents.push(rename);
            parents.sort();
            parents.dedup();
            out.push(Event { id: derived_id(rename, &op), parents, op, links: Vec::new() });
        }
    }
    out
}
