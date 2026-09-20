//! argv in, exit code out. No logic lives here.
//!
//! ```text
//! v0 init                     v0 log                 v0 sync <other-repo>
//! v0 record -m MESSAGE        v0 adopt <change>      v0 drop <change>
//! v0 status                   v0 deps <change>       v0 reduction
//! ```
//!
//! Exit codes, following the lesson from the pijul harness — a dependency
//! violation is a *state* to handle, not a crash:
//!
//! | code | meaning |
//! |------|---------|
//! | 0 | success |
//! | 1 | failure |
//! | 2 | usage |
//! | 3 | change set not dependency-closed |
fn main() {
    todo!()
}
