//! argv in, exit code out. No logic lives here.
//!
//! Exit codes, following the lesson from the pijul harness — a dependency
//! violation is a *state* to handle, not a crash:
//!
//! | code | meaning |
//! |------|---------|
//! | 0 | success |
//! | 1 | failure |
//! | 2 | usage |
//! | 3 | dependency violation (the set was not closed) |
fn main() {
    todo!()
}
