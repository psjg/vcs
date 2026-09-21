//! Memory the TigerStyle way: a budget fixed at startup, never exceeded.
//!
//! TigerBeetle's style guide asks that all memory be allocated at startup and
//! that everything have a limit (<https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md>).
//! v0 applies it at the level of the process: the operating system is asked
//! for memory at startup and never again. Every allocation after that comes out
//! of one reserved region, and running out is a loud, immediate failure with
//! its own exit code — never a machine grinding to a halt in swap.
//!
//! How it came to be: a test once expanded a four-billion-element delete range
//! and ran the developer's Mac out of memory. The range is now clamped
//! (`op::clamp`); this module is the net under every bug of that shape.
//!
//! Three stages:
//!
//! 1. **Bootstrap** — the very first allocation of the process, before `main`,
//!    claims a small fixed block so the runtime and argument parsing can run.
//! 2. **Reserve** — `main` calls [`reserve_mb`] with `--memory`, `V0_MEMORY_MB`
//!    or [`DEFAULT_MB`], and the rest of the budget is claimed in one piece.
//!    Binaries that never call it (tests, tools) reserve lazily on first need,
//!    from `V0_MEMORY_MB` or the default — bounded either way.
//! 3. **Exhausted** — any further request prints why and exits with code 5.
//!
//! What it does **not** cover, as with the JVM's `-Xmx`: thread stacks and the
//! binary itself. It bounds the heap, which is where runaway growth happens.
//!
//! Not full TigerStyle: TigerBeetle also never allocates *within* its budget
//! after startup, because every structure has a fixed capacity. v0's data model
//! still changes daily, so inside the region it allocates and frees freely.


/// Budget when nothing says otherwise.
pub const DEFAULT_MB: usize = 512;
const MB: usize = 1 << 20;
/// Enough for the runtime and argument parsing, before the budget is known.
const BOOTSTRAP: usize = 2 * MB;
/// Exit code for "the budget ran out" — distinct, so scripts and agents can
/// branch on it.
pub const EXIT_EXHAUSTED: i32 = 5;

pub use imp::{limit_bytes, reserve_mb};

/// The budget itself: talc over one region reserved at startup.
#[cfg(not(feature = "system-alloc"))]
mod imp {
    use super::{BOOTSTRAP, DEFAULT_MB, EXIT_EXHAUSTED, MB};
    use std::alloc::{GlobalAlloc, Layout, System};
    use talc::base::Talc;
    use talc::base::binning::Binning;
    use talc::source::Source;

    #[derive(Debug)]
    pub struct Budget {
        stage: Stage,
        /// The whole budget in bytes, bootstrap included. Zero until known.
        total: usize,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Stage {
        Empty,
        Bootstrapped,
        Reserved,
    }

    impl Budget {
        pub const fn new() -> Self {
            Self { stage: Stage::Empty, total: 0 }
        }
    }

    impl Default for Budget {
        fn default() -> Self {
            Self::new()
        }
    }

    /// The process-wide allocator: talc over the reserved region.
    #[global_allocator]
    static ALLOC: talc::TalcLock<spinning_top::RawSpinlock, Budget> = talc::TalcLock::new(Budget::new());

    // SAFETY: `acquire` only ever touches `talc` through the reference it is given,
    // asks the *system* allocator (never the global one) for regions, and on
    // exhaustion writes to stderr and exits without allocating.
    unsafe impl Source for Budget {
        fn acquire<B: Binning>(talc: &mut Talc<Self, B>, layout: Layout) -> Result<(), ()> {
            match talc.source.stage {
                Stage::Empty => {
                    talc.source.total = env_budget().max(BOOTSTRAP);
                    claim(talc, BOOTSTRAP)?;
                    talc.source.stage = Stage::Bootstrapped;
                    Ok(())
                }
                // Nobody called `reserve_mb`: take the rest of the budget now.
                Stage::Bootstrapped => {
                    let rest = talc.source.total - BOOTSTRAP;
                    talc.source.stage = Stage::Reserved;
                    if rest > 0 { claim(talc, rest) } else { exhausted(talc.source.total, layout.size()) }
                }
                Stage::Reserved => exhausted(talc.source.total, layout.size()),
            }
        }
    }

    /// Fix the budget and claim all of it, now. Call once, first thing in `main`.
    ///
    /// Fails if the budget was already fixed — which only happens if startup
    /// needed more than the bootstrap block — or is smaller than the bootstrap.
    pub fn reserve_mb(mb: usize) -> Result<(), &'static str> {
        let total = mb.saturating_mul(MB);
        if total < BOOTSTRAP {
            return Err("memory budget must be at least 2 MB");
        }
        // Nothing below may allocate: the allocator's own lock is held.
        let mut talc = ALLOC.lock();
        match talc.source.stage {
            Stage::Reserved => Err("memory budget was already fixed before it could be set"),
            Stage::Empty | Stage::Bootstrapped => {
                if talc.source.stage == Stage::Empty {
                    claim(&mut talc, BOOTSTRAP).map_err(|_| "could not reserve memory")?;
                }
                talc.source.total = total;
                talc.source.stage = Stage::Reserved;
                let rest = total - BOOTSTRAP;
                if rest > 0 {
                    claim(&mut talc, rest).map_err(|_| "could not reserve memory")?;
                }
                Ok(())
            }
        }
    }

    /// The budget in bytes, once known.
    pub fn limit_bytes() -> usize {
        ALLOC.lock().source.total
    }

    /// `V0_MEMORY_MB`, or the default. Read with `getenv` because `std::env`
    /// allocates, and this runs inside the allocator.
    fn env_budget() -> usize {
        // SAFETY: a NUL-terminated literal; getenv returns null or a C string.
        let raw = unsafe { libc::getenv(c"V0_MEMORY_MB".as_ptr()) };
        if raw.is_null() {
            return DEFAULT_MB * MB;
        }
        let mut mb: usize = 0;
        let mut p = raw;
        // SAFETY: walking a C string up to its terminator.
        unsafe {
            while (*p as u8).is_ascii_digit() {
                mb = mb.saturating_mul(10).saturating_add((*p as u8 - b'0') as usize);
                p = p.add(1);
            }
        }
        if mb == 0 { DEFAULT_MB * MB } else { mb.saturating_mul(MB) }
    }

    /// Ask the *system* allocator for a region and hand it to talc.
    fn claim<B: Binning>(talc: &mut Talc<Budget, B>, size: usize) -> Result<(), ()> {
        let layout = Layout::from_size_align(size, 4096).map_err(|_| ())?;
        // SAFETY: nonzero size, valid alignment; the region is never freed, which
        // is the point -- it is the process's whole heap.
        let base = unsafe { System.alloc(layout) };
        if base.is_null() {
            return Err(());
        }
        // SAFETY: `base` is a fresh, exclusively owned region of `size` bytes.
        unsafe { talc.claim(base, size) }.map(|_| ()).ok_or(())
    }

    /// Say why, then stop. No allocation: the message is assembled on the stack and
    /// written with a raw `write(2)`, because the allocator is the thing that failed.
    fn exhausted(total: usize, wanted: usize) -> ! {
        let mut buf = [0u8; 256];
        let mut n = 0;
        let mut put = |bytes: &[u8]| {
            for b in bytes {
                if n < buf.len() {
                    buf[n] = *b;
                    n += 1;
                }
            }
        };
        put(b"v0: memory budget of ");
        put(digits(total / MB, &mut [0u8; 20]));
        put(b" MB exhausted (asked for ");
        put(digits(wanted, &mut [0u8; 20]));
        put(b" more bytes). Everything has a limit; raise this one with --memory or V0_MEMORY_MB.\n");
        // SAFETY: writing an initialised stack buffer to stderr, then exiting
        // without running destructors that might allocate.
        unsafe {
            libc::write(2, buf.as_ptr().cast(), n);
            libc::_exit(EXIT_EXHAUSTED);
        }
    }

    /// Decimal digits of `v`, without allocating.
    fn digits(mut v: usize, out: &mut [u8; 20]) -> &[u8] {
        let mut i = out.len();
        loop {
            i -= 1;
            out[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        &out[i..]
    }
}

/// Profiling build: the system allocator, no budget.
///
/// The TigerStyle allocator serves everything out of one region, so Instruments
/// Allocations, `heap`, `leaks` and `malloc_history` see a single giant block
/// and nothing inside it. To find *which structure* is using memory, build with
/// `--features system-alloc` and record with
/// `xcrun xctrace record --template 'Allocations' --launch -- target/release/v0 ...`.
/// Never ship this: it removes the limit.
#[cfg(feature = "system-alloc")]
mod imp {
    pub fn reserve_mb(_mb: usize) -> Result<(), &'static str> {
        Ok(())
    }

    /// Zero: there is no budget in a profiling build.
    pub fn limit_bytes() -> usize {
        0
    }
}
