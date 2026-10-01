#!/bin/bash
# The slow and reference-dependent tests (all #[ignore]d in the normal suite), release build.
#   heavy-tests.sh          statistical validation (exact and fast mode), open-bug tests,
#                           generator-on-reference check, exact-vs-fast equivalence campaign
#   heavy-tests.sh --fuzz   also a 200-model differential fuzz campaign against the reference
set -u
cd "$(dirname "$0")/../../.."
export PATH=$HOME/.cargo/bin:$PATH
st=0
cargo test --release -p simoxide-sim --test statistical -- --ignored --nocapture || st=1
cargo test --release -p simoxide-testkit --test modelgen -- --ignored || st=1
cargo test --release -p simoxide-sim --test bugs -- --ignored || echo "(regression tests: docs/correctness/testing.md)"
# fast mode: statistical equivalence with the exact mode (docs/correctness/testing.md, "Fast mode")
cargo build --release -p simoxide-testkit || exit 1
./target/release/simoxide-fuzz equiv --quiet --top 10 --seeds 40 \
  --corpus corpus,corpus-fuzz,crates/simoxide-sim/tests/models,crates/simoxide-cli/bench/models || st=1
./target/release/simoxide-fuzz equiv --quiet --top 10 --seeds 40 --gen 200 --seed "${EQUIV_SEED:-1}" \
  --sizes 1..10 || st=1
if [ "${1:-}" = "--fuzz" ]; then
  cargo build --release -p simoxide-cli -p simoxide-testkit || exit 1
  ./target/release/simoxide-fuzz fuzz --n 200 --seed "${FUZZ_SEED:-$RANDOM}" --sizes 1..10 --save-ref-failures \
    --sim "cmd:$PWD/target/release/simoxide run --model {dir} --run-json {run_json} --name {name} --trace {trace} --tape {tape} --measurements {measurements} {replay:--replay-tape}" || st=1
fi
exit $st
