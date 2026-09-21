#!/bin/sh
# Sweep the live weave's two size knobs -- the sum tree's branching factor B
# and the fragment cap F -- across document sizes, three runs each.
#
#   bench/weave.sh TEXT-FILE > results.csv
#
# Each (B, F) is its own build: they are compile-time constants, read from
# V0_SUMTREE_B and V0_MAX_FRAGMENT. Run inside the rust profile.
set -eu
text=$1
cd "$(dirname "$0")/.."
echo "b,f,bytes,events,fragments,seek_ns,make_ns,apply_ns,typed_p50_ns,typed_p99_ns,typed_max_ns,back_p50_ns,back_p99_ns,report_ns,replay_ms,from_walk_ms,move_us,append_us"
for b in 8 16 32 64; do
  for f in 32 128 512; do
    V0_SUMTREE_B=$b V0_MAX_FRAGMENT=$f cargo build --quiet --release --example weave_bench
    for bytes in 20000 200000 2000000; do
      for run in 1 2 3; do
        ./target/release/examples/weave_bench "$text" "$bytes" 5000 2000
      done
    done
  done
done
