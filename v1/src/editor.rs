//! The editor's side: an LSP client (Helix, VS Code, Neovim's built-in)
//! speaking to us as its language server, translated to and from
//! [`v0::live`]'s messages. Pure: no I/O, so a fake client drives it in a
//! test. [`Front`] plays the part `editors/nvim` and `editors/emacs` play
//! for `v0 live`; the session counts in bytes ([`v0::live::Unit::Bytes`]).
//!
//! # The protocol, measured on Helix 25.07 (2026-11-11)
//!
//! - `workspace/applyEdit` carrying a `textDocument.version` the client has
//!   moved past is refused: `{applied: false, failureReason: "document has
//!   changed"}`. One carrying the current version is applied.
//! - An applied edit comes back as a `didChange` at version + 1, *before*
//!   the `applyEdit` response. That `didChange` is the echo, not typing.
//!   LSP promises neither: a client may answer first, and versions need
//!   only increase, which [`Front::changed`] checks of every `didChange`,
//!   so the echo is any later version.
//! - Without a version an edit is applied unchecked: never send one.
//! - Helix offers position encodings utf-8, utf-32 and utf-16.
//!
//! So there is at most one push in flight. `didChange`s that arrive while
//! one is are queued; when the answer comes, an applied push's first queued
//! change is its echo (it confirms: [`FromEditor::Seen`]) and the rest are
//! typing, against the pushed state (an applied push answered before its
//! echo came waits for it: the next `didChange` is the echo); a refused
//! push confirms nothing and
//! every queued change is typing against the state before it -- an edit
//! typed behind, which the session reads where it was typed. Session
//! messages that arrive meanwhile wait their turn; when one is taken, the
//! `acked` rule of [`v0::live::ToEditor::Apply`] decides whether it still
//! fits, as a plugin would.
//!
//! # Errors
//!
//! A reader that guesses hides a desynchronised client. So a position
//! outside the mirror (past a line's end, inside a character, a line that
//! is not there), a `didChange` whose version does not advance, an echo
//! that is not the push, an answer with no push in flight, or a session
//! `Error` is an `Err`, never a clamp. After an `Err` the mirror is not to
//! be trusted: the caller reports it and reopens the document.
//! [`Front::rename`] and [`Front::offset`] are refused while a push is in
//! flight, since the client's coordinates are then ambiguous: its buffer
//! is the mirror or the pushed text, whichever the answer turns out to say.
//!
//! Lines end at `\n`, `\r\n` or `\r`; a column never includes the
//! terminator.

use lsp_types::{Position, PositionEncodingKind, Range, TextDocumentContentChangeEvent, TextEdit};
use std::collections::VecDeque;
use v0::live::{FromEditor, Patch, ToEditor};

/// How the client counts columns: negotiated at `initialize`. utf-8 when
/// the client offers it (Helix, Neovim), else utf-16 (the default, VS Code).
/// utf-32 is never chosen: utf-16 serves whoever offers it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encoding {
    Utf8,
    Utf16,
}

impl Encoding {
    /// The encoding to choose from what a client offers.
    pub fn choose(offered: Option<&[PositionEncodingKind]>) -> Encoding {
        match offered {
            Some(kinds) if kinds.contains(&PositionEncodingKind::UTF8) => Encoding::Utf8,
            _ => Encoding::Utf16,
        }
    }

    /// What to answer in `InitializeResult`.
    pub fn kind(self) -> PositionEncodingKind {
        match self {
            Encoding::Utf8 => PositionEncodingKind::UTF8,
            Encoding::Utf16 => PositionEncodingKind::UTF16,
        }
    }

    /// A character's width in this encoding's code units.
    fn width(self, c: char) -> usize {
        match self {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
        }
    }
}

/// An edit to push to the client: `workspace/applyEdit` on the live
/// document at `version`, the version the client is believed to be at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Push {
    pub version: i32,
    pub edits: Vec<TextEdit>,
}

/// The push in flight: what the client shows if it applies it.
struct Flight {
    /// The session message it confirms; none for a guest's first fill.
    n: Option<u64>,
    text: String,
    /// The client answered that it applied the push, and its echo has not
    /// come: the next `didChange` is it.
    applied: bool,
}

/// One live document as the client shows it.
pub struct Front {
    encoding: Encoding,
    /// The client's buffer as last known, and its version.
    mirror: String,
    version: i32,
    /// Edits and renames sent to the session: the `acked` a message must
    /// carry to fit.
    sent: u64,
    /// The last `Apply` the client confirmed.
    seen: u64,
    flight: Option<Flight>,
    /// `didChange`s that came while a push was in flight, oldest first.
    queue: Vec<(i32, Vec<TextDocumentContentChangeEvent>)>,
    /// Session messages that came meanwhile, oldest first.
    waiting: VecDeque<ToEditor>,
    /// A guest's buffer is not the session's document until its first fill
    /// lands: the text of the fill. What is typed before that is not sent;
    /// it goes with the buffer it was typed into.
    fill: Option<String>,
}

/// What the front has for the two sides after an event.
#[derive(Default, Debug)]
pub struct Out {
    pub session: Vec<FromEditor>,
    pub push: Option<Push>,
}

impl Front {
    /// The document as `didOpen` gave it. For a host, the session then
    /// needs this text as its first edit ([`Front::opened`]).
    pub fn new(encoding: Encoding, text: String, version: i32) -> Front {
        Front {
            encoding,
            mirror: text,
            version,
            sent: 0,
            seen: 0,
            flight: None,
            queue: Vec::new(),
            waiting: VecDeque::new(),
            fill: None,
        }
    }

    /// A host's first message: the whole buffer typed into an empty
    /// document. A guest instead replaces the buffer with the session's
    /// text: [`Front::heard`] of [`ToEditor::Ready`] pushes it.
    pub fn opened(&mut self) -> Vec<FromEditor> {
        if self.mirror.is_empty() {
            return Vec::new();
        }
        self.sent += 1;
        vec![FromEditor::Edit { at: 0, del: 0, text: self.mirror.clone(), seen: self.seen }]
    }

    /// The client's `didChange`. Its changes are sequential, each in the
    /// coordinates the one before left; one without a range replaces the
    /// whole document. Typing goes to the session at once, unless a push
    /// is in flight: then it waits for the answer, which says what it was
    /// typed against.
    pub fn changed(&mut self, version: i32, changes: Vec<TextDocumentContentChangeEvent>) -> Result<Out, String> {
        let known = self.queue.last().map_or(self.version, |(v, _)| *v);
        if version <= known {
            return Err(format!("didChange version {version} does not follow {known}"));
        }
        let mut out = Out::default();
        match &self.flight {
            Some(f) if f.applied => {
                self.land(version, changes, &mut out)?;
                self.resume(Vec::new(), &mut out)?;
            }
            Some(_) => self.queue.push((version, changes)),
            None => self.typed(version, changes, &mut out)?,
        }
        Ok(out)
    }

    /// A message from the session. An `Apply` becomes a push when it is
    /// its turn and still fits the client's buffer; one that does not is
    /// dropped, as a plugin drops it. A session `Error` is returned.
    pub fn heard(&mut self, msg: ToEditor) -> Result<Out, String> {
        if let ToEditor::Error(why) = msg {
            return Err(format!("the session refused: {why}"));
        }
        self.waiting.push_back(msg);
        let mut out = Out::default();
        self.pump(&mut out)?;
        Ok(out)
    }

    /// The client's answer to the push in flight.
    ///
    /// Applied: the first queued `didChange` is its echo, which must be the
    /// push itself, at any later version; the session is told
    /// [`FromEditor::Seen`], and the rest was typed against the pushed
    /// text. With none queued the echo is still to come, and the push stays
    /// in flight until [`Front::changed`] gets it. Refused: nothing is
    /// confirmed, everything queued was typed against the text as it was,
    /// and the session messages waiting are dropped: the session is asked
    /// for what the buffer lacks, by what was typed or by an edit of
    /// nothing.
    pub fn answered(&mut self, applied: bool) -> Result<Out, String> {
        let flight = self.flight.as_mut().filter(|f| !f.applied).ok_or("an answer with no push in flight")?;
        let mut queue = std::mem::take(&mut self.queue);
        let mut out = Out::default();
        // A message refused: those waiting are relative to it, so they do
        // not fit the buffer, whatever `acked` says of what was typed.
        let lost = !applied && flight.n.is_some();
        if !applied {
            self.flight = None;
        } else if queue.is_empty() {
            flight.applied = true;
            return Ok(out);
        } else {
            let (version, changes) = queue.remove(0);
            self.land(version, changes, &mut out)?;
        }
        if lost {
            self.waiting.clear();
        }
        let sent = self.sent;
        self.resume(queue, &mut out)?;
        // Nothing typed to ask the session what the buffer lacks: ask with
        // an edit of nothing, which it answers as one typed behind.
        if lost && self.sent == sent {
            self.sent += 1;
            out.session.push(FromEditor::Edit { at: 0, del: 0, text: String::new(), seen: self.seen });
        }
        Ok(out)
    }

    /// The echo of the push in flight, `didChange` `version`: it must be
    /// the push. The mirror becomes the pushed text, and the session is
    /// told the message it carried was seen.
    fn land(&mut self, version: i32, changes: Vec<TextDocumentContentChangeEvent>, out: &mut Out) -> Result<(), String> {
        let flight = self.flight.take().ok_or("an echo with no push in flight")?;
        let mut echoed = self.mirror.clone();
        for c in changes {
            splice(self.encoding, &mut echoed, c)?;
        }
        if echoed != flight.text {
            return Err("the echo is not the push".to_owned());
        }
        self.mirror = flight.text;
        self.version = version;
        self.fill = None;
        if let Some(n) = flight.n {
            self.seen = n;
            out.session.push(FromEditor::Seen(n));
        }
        Ok(())
    }

    /// With no push in flight: the `queue` of `didChange`s typed meanwhile,
    /// then what the session has waiting.
    fn resume(&mut self, queue: Vec<(i32, Vec<TextDocumentContentChangeEvent>)>, out: &mut Out) -> Result<(), String> {
        for (version, changes) in queue {
            self.typed(version, changes, out)?;
        }
        // A refused fill: push again, against what the buffer is now.
        if let Some(text) = self.fill.clone() {
            self.push(None, text, out);
        }
        self.pump(out)
    }

    /// `textDocument/rename` at `at`, in the client's coordinates, as an
    /// intent for the session. Refused while a push is in flight.
    pub fn rename(&mut self, at: Position, to: String) -> Result<FromEditor, String> {
        if self.flight.is_some() {
            return Err("a push is in flight: the buffer's coordinates are not known".to_owned());
        }
        let at = self.offset(at).ok_or_else(|| format!("{at:?} is not in the buffer"))?;
        self.sent += 1;
        Ok(FromEditor::Rename { at, to, seen: self.seen })
    }

    /// The code action of `kind` and `title` over `range`, in the client's
    /// coordinates, as an intent for the session. Refused while a push is
    /// in flight.
    pub fn action(&mut self, range: Range, kind: String, title: String) -> Result<FromEditor, String> {
        let (at, end) = self.span(range)?;
        self.sent += 1;
        Ok(FromEditor::Action { at, end, kind, title, seen: self.seen })
    }

    /// Format `range`, in the client's coordinates, as an intent for the
    /// session. Refused while a push is in flight.
    pub fn format(&mut self, range: Range) -> Result<FromEditor, String> {
        let (at, end) = self.span(range)?;
        self.sent += 1;
        Ok(FromEditor::Format { at, end, seen: self.seen })
    }

    /// The whole buffer as a range, in the client's coordinates.
    pub fn whole(&self) -> Range {
        let end = position(self.encoding, &self.mirror, self.mirror.len()).expect("the end of the text is a position");
        Range::new(Position::new(0, 0), end)
    }

    /// The client position of byte `at` of the buffer as the client shows
    /// it. None when `at` is not a character boundary of it, or inside a
    /// line terminator.
    pub fn position(&self, at: usize) -> Option<Position> {
        position(self.encoding, &self.mirror, at)
    }

    /// `at`, in the client's coordinates, as a byte offset of the buffer as
    /// the client shows it. For requests passed through to the backend.
    /// None when it is not in the buffer, or while a push is in flight.
    pub fn offset(&self, at: Position) -> Option<usize> {
        if self.flight.is_some() {
            return None;
        }
        byte(self.encoding, &self.mirror, at)
    }

    /// How the client counts columns.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The buffer as the client is known to show it.
    pub fn text(&self) -> &str {
        &self.mirror
    }

    /// A range of the client as byte offsets of the mirror, start first.
    fn span(&self, r: Range) -> Result<(usize, usize), String> {
        if self.flight.is_some() {
            return Err("a push is in flight: the buffer's coordinates are not known".to_owned());
        }
        let at = |p: Position| byte(self.encoding, &self.mirror, p).ok_or_else(|| format!("{p:?} is not in the buffer"));
        let (start, end) = (at(r.start)?, at(r.end)?);
        if end < start {
            return Err(format!("range {r:?} ends before it starts"));
        }
        Ok((start, end))
    }

    /// Typing: change the mirror and tell the session, unless the buffer
    /// is not yet the session's.
    fn typed(&mut self, version: i32, changes: Vec<TextDocumentContentChangeEvent>, out: &mut Out) -> Result<(), String> {
        for c in changes {
            let Some((at, del, text)) = splice(self.encoding, &mut self.mirror, c)? else { continue };
            if self.fill.is_none() {
                self.sent += 1;
                out.session.push(FromEditor::Edit { at, del, text, seen: self.seen });
            }
        }
        self.version = version;
        Ok(())
    }

    /// Take waiting session messages in order, until one needs a push.
    fn pump(&mut self, out: &mut Out) -> Result<(), String> {
        while self.flight.is_none() {
            match self.waiting.pop_front() {
                None => break,
                Some(ToEditor::Error(why)) => return Err(format!("the session refused: {why}")),
                Some(ToEditor::Ready { role, text }) => {
                    if role != "host" {
                        self.fill = Some(text.clone());
                        self.push(None, text, out);
                    }
                }
                // The editor typed since: the message was not meant for this buffer.
                Some(ToEditor::Apply { acked, .. }) if acked != self.sent => {}
                Some(ToEditor::Apply { n, edits, .. }) => {
                    let mut text = self.mirror.clone();
                    for p in &edits {
                        apply(&mut text, p)?;
                    }
                    self.push(Some(n), text, out);
                }
            }
        }
        Ok(())
    }

    /// Bring the client from the mirror to `text`: one replacement of what
    /// differs, as a push; or, with nothing to change, the confirmation.
    fn push(&mut self, n: Option<u64>, text: String, out: &mut Out) {
        let Some((mut at, mut del, mut new_text)) = diff(&self.mirror, &text) else {
            self.fill = None;
            if let Some(n) = n {
                self.seen = n;
                out.session.push(FromEditor::Seen(n));
            }
            return;
        };
        // The middle of a `\r\n` is no position: the edit takes it whole.
        let m = self.mirror.as_bytes();
        if at > 0 && m[at - 1] == b'\r' && m.get(at) == Some(&b'\n') {
            (at, del) = (at - 1, del + 1);
            new_text.insert(0, '\r');
        }
        if del > 0 && m[at + del - 1] == b'\r' && m.get(at + del) == Some(&b'\n') {
            del += 1;
            new_text.push('\n');
        }
        let place = |k| position(self.encoding, &self.mirror, k).expect("an edit starts and ends at a position");
        let (start, end) = (place(at), place(at + del));
        out.push = Some(Push { version: self.version, edits: vec![TextEdit { range: Range::new(start, end), new_text }] });
        self.flight = Some(Flight { n, text, applied: false });
    }
}

/// The session's patch, in bytes, applied to `text`. A patch outside the
/// text, or inside a character, is an error.
fn apply(text: &mut String, p: &Patch) -> Result<(), String> {
    let end = p.at.checked_add(p.del).filter(|e| *e <= text.len());
    match end {
        Some(end) if text.is_char_boundary(p.at) && text.is_char_boundary(end) => {
            text.replace_range(p.at..end, &p.text);
            Ok(())
        }
        _ => Err(format!("patch {}+{} is not in the buffer", p.at, p.del)),
    }
}

/// One content change applied to `text`: the byte replacement `(at, del,
/// text)` it made, none when it changed nothing (a whole-document change
/// to the same text).
fn splice(
    enc: Encoding,
    text: &mut String,
    change: TextDocumentContentChangeEvent,
) -> Result<Option<(usize, usize, String)>, String> {
    let edit = match change.range {
        None => diff(text, &change.text),
        Some(r) => {
            let at = byte(enc, text, r.start).ok_or_else(|| format!("{:?} is not in the buffer", r.start))?;
            let end = byte(enc, text, r.end).ok_or_else(|| format!("{:?} is not in the buffer", r.end))?;
            if end < at {
                return Err(format!("range {r:?} ends before it starts"));
            }
            Some((at, end - at, change.text))
        }
    };
    if let Some((at, del, new)) = &edit {
        text.replace_range(*at..*at + *del, new);
    }
    Ok(edit)
}

/// The one replacement `(at, del, text)` that turns `a` into `b`, in bytes:
/// what differs between their common prefix and suffix. None if equal.
fn diff(a: &str, b: &str) -> Option<(usize, usize, String)> {
    if a == b {
        return None;
    }
    let head: usize = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum();
    let (ta, tb) = (&a[head..], &b[head..]);
    let tail: usize = ta.chars().rev().zip(tb.chars().rev()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum();
    Some((head, ta.len() - tail, tb[..tb.len() - tail].to_owned()))
}

/// The lines of `text` as `(start, end, next)`: `end` is before the
/// terminator, `next` after it. Always one more than the terminators.
fn lines(text: &str) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let b = text.as_bytes();
    let (mut start, mut done) = (0, false);
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let end = (start..b.len()).find(|&i| b[i] == b'\n' || b[i] == b'\r').unwrap_or(b.len());
        let next = match b.get(end) {
            None => b.len(),
            Some(b'\r') if b.get(end + 1) == Some(&b'\n') => end + 2,
            Some(_) => end + 1,
        };
        done = end == b.len();
        let line = (start, end, next);
        start = next;
        Some(line)
    })
}

/// A client position as a byte offset of `text`; None for a line that is
/// not there, a column past the line's end or inside a character. Lines
/// end at `\n`, `\r\n` or `\r`, in the client's buffer and in a language
/// server's alike ([`crate::backend::Backend`] reads positions so too).
pub fn byte(enc: Encoding, text: &str, at: Position) -> Option<usize> {
    let (start, end, _) = lines(text).nth(at.line as usize)?;
    let mut col = 0usize;
    for (i, c) in text[start..end].char_indices() {
        if col == at.character as usize {
            return Some(start + i);
        }
        if col > at.character as usize {
            return None;
        }
        col += enc.width(c);
    }
    (col == at.character as usize).then_some(end)
}

/// The client position of byte `offset` of `text`; None when it is not a
/// character boundary of it, or is inside a line terminator.
pub fn position(enc: Encoding, text: &str, offset: usize) -> Option<Position> {
    if !text.is_char_boundary(offset) {
        return None;
    }
    for (n, (start, end, next)) in lines(text).enumerate() {
        if offset <= end {
            return Some(Position::new(n as u32, text[start..offset].chars().map(|c| enc.width(c)).sum::<usize>() as u32));
        }
        if offset < next {
            return None;
        }
    }
    None
}
