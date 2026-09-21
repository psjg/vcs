# Shelved: prettier history, removable secrets, provenance

**Status:** shelved 2026-09-21, from a design conversation during the v0 spike.
Nothing here is decided or built. Kept at the repository root rather than in
`v0/` because it is likely to outlive v0 — a later version may pick it up.

**To resume:** read *Where it stands*, then *Resume here*.

## The two wishes

On top of v0's "history does not lie" model (nothing rewritten; a conflict
resolution is an ordinary change; which side survived is derived, not claimed),
two things are wanted:

1. **Make history look better than it was**, without the underlying record
   lying.
2. **Make leaked sensitive content really not travel** — API keys, tokens,
   trade secrets — even though v0 may replicate to peers live. Possibly
   impossible in the limit; get as close as possible.

These are different in kind: the first is *presentation* (non-destructive), the
second is *destruction* (audited). Keep them in separate layers. Git's
`filter-branch`/`filter-repo` does both at once, badly, by rewriting.

## 1. Presentation: a narrative layer

In v0 a change is already a label over events. Prettier history therefore needs
no rewriting — only **another layer of labels above changes**: a *narrative*
groups changes, orders them for reading, and gives them clean messages.

- It is its own content-addressed, append-only object; a newer narrative
  supersedes an older one.
- `log` shows the narrative; `log --raw` shows the truth.
- `adopt`, `drop` and `deps` keep operating on real changes, so a narrative
  cannot break anything.
- It lies only in presentation, and is itself recorded (who, when).

**Not optional in practice.** ADR-0007 (split `record` by dependency component)
turned fpl's 76 commits into ~200 changes and splits a fix from the test that
covers it. The narrative puts back the units a human calls "a commit".
*Machine-atomic for dependencies, human-atomic for reading* — which also
resolves the reference-versus-intent tension noted in ADR-0007.

Related prior art to read before building: jj's evolution log (history of a
change's rewrites), Fossil's append-only amendments.

## 2. Destruction: sensitive content

### The honest limit

GitHub's own guidance is to consider a pushed secret **compromised and rotate it
first**, and it warns that rewriting history risks recontamination from older
clones. Bits that reached a peer cannot be recalled. v0 cannot beat that, so
**rule one is still: rotate.**

### What v0 can do better than git

**a. Stop it at capture, before sync.** The strongest lever, and in a live system
the only one that is on time. An off-the-shelf scanner (gitleaks, trufflehog) as
a hard gate in `record` *and* `sync`, plus a tunable quarantine — events stay
local until scanned. An explicit trade against liveness.

**b. Redact without breaking the graph — already true by accident.** Verified in
the code on 2026-09-21: `EventId` is positional (`seq`, `replica`), and
`ChangeId` hashes event ids, deps and meta — **not line text**. A line's content
can be blanked in place while every id, anchor, dependency and conflict stays
valid. Git cannot do this: content is hash-addressed, so removing one blob
rewrites every descendant hash.

The same fact is a **hole**: nothing currently verifies line content. The fix for
both is to **separate structure from content** — an event commits to
`hash(content)`, content lives in a blob store. Redaction drops the blob and
keeps the hash; the line materialises as `[redacted: reason]`. Integrity for
what you hold, id-stable redaction for what you must not.

**c. Replicated obliteration, but authorised.** Fossil's *shunning* list
deliberately does **not** propagate by sync: a deletion that spreads is a weapon —
a malicious user or a bug could destroy every repository in a network. (A pull
does receive it by default, via `auto-shun`.) For v0: a **signed** `Obliterate`
event, which each peer honours according to its own policy for identities it
trusts. Honest peers purge; dishonest ones cannot be forced. A tombstone remains
— *something was here, redacted by X on Y because Z* — so even destruction does
not lie about the fact of history, only removes its content.

## 3. Cryptography

### Crypto-shredding (secrecy that can be revoked — partly)

The idea as first stated — "encrypt with a private key, decrypt with a published
public key" — is **signing**, not secrecy: everyone holds the public key. What is
wanted is the known event-sourcing pattern **crypto-shredding**:

- content encrypted with a **symmetric** key per change (e.g.
  XChaCha20-Poly1305, off the shelf);
- that key **wrapped** for each authorised reader's public key — where public-key
  crypto properly belongs, as `age` does with recipients;
- obliterate = **destroy the key**. The ciphertext may stay everywhere; it is
  dead.

The limit, stated sharply: a key can only be destroyed while it has **never left
its keeper**. A peer that unwrapped it and decrypted has plain bits in memory,
and no cryptography recalls those. So it works for backups, mirrors, relays and
every peer that joins later; it does not work for a peer that already read the
content — including your own plaintext working copy.

The only way to refuse readers *after the fact* is a keyholder in every read
path. That is DRM, and no longer peer-to-peer. **Revocable secrecy and
peer-to-peer do not go together.** The cost is not the amount of cryptography —
that is little code, off the shelf — but **key management**: who gets which key,
rotation, what a new peer may see.

### Signing (provenance)

Signing is cheap and gives the automatic identity provenance that was the
original intuition.

- **Git only half does this.** `author` and `committer` are plain strings anyone
  can set. Signing (GPG, and SSH keys since Git 2.34) is opt-in, and so is
  verification.
- **Pijul does it automatically:** every change is signed with the author's
  identity key, which is why `pijul identity new` is required first.
- **v0 today:** `meta.author` is `$USER` — as unauthenticated as git's
  `user.name`.

Two constraints for v0:

1. **Sign changes (or sync batches), not events.** Signing every keystroke in a
   live system wastes the work.
2. **A signature over `ChangeId` would not cover the text**, because `ChangeId`
   does not hash content. A peer could alter a line under a valid signature. Git
   avoids this because a commit covers blobs through the tree hash. The
   structure/content split in 2b gives v0 the same chain.

## Where it stands

Everything above converges on **one foundation: separating structure from
content** (events commit to `hash(content)`, content in a blob store). It is the
prerequisite for id-stable redaction (2b), content integrity (2b), crypto-shredding
(3), and signatures that cover the text (3). It gets more expensive the more is
built on the current format.

## Resume here

Suggested order, when this comes back:

1. Structure/content split — the foundation, and cheapest early.
2. Signing changes — nearly free once (1) exists.
3. Narrative layer — independent of the rest; needed as soon as anyone reads a
   v0 history made of component-split changes.
4. Scanner gate + sync quarantine — prevention, the strongest lever.
5. Signed, policy-honoured `Obliterate` with tombstones.
6. Crypto-shredding — last, and only if the threat model justifies the
   key-management cost.

Open questions:

- What does a narrative look like when two people narrate the same changes
  differently? (A conflict about presentation — probably allowed to coexist.)
- Who may sign an `Obliterate` that others honour, and how does that trust get
  bootstrapped without a server?
- Quarantine length versus liveness: is "scan before the event leaves the
  machine" fast enough to be invisible?

## Sources

- [Fossil: Deleting Content From Fossil (shunning)](https://fossil-scm.org/home/doc/trunk/www/shunning.wiki)
- [GitHub Docs: Removing sensitive data from a repository](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/removing-sensitive-data-from-a-repository)
- v0 source: `v0/src/change.rs` (`Change`, `canonical`), `v0/src/op.rs` (`EventId`)
