#!/usr/bin/env bash
# Long-run regression check for performance work: runs every corpus / test / bench model with
# larger stop conditions (own RNG) and prints a hash of trace + tape + measurements per run.
#   bench/golden.sh [simoxide binary] > new.txt ; diff old.txt new.txt
# GOLDEN_EXTRA=/dir adds every model directory below /dir (e.g. from `simoxide-fuzz gen`);
# GOLDEN_JOBS sets the parallelism (default 8).
set -u
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
export BIN="${1:-$ROOT/target/release/simoxide}"
TMP="$(mktemp -d)"
export TMP
trap 'rm -rf "$TMP"' EXIT
one() {
  d="$1"; cfg="$2"
  w="$(mktemp -d -p "$TMP")"
  # shellcheck disable=SC2086
  if "$BIN" run --model "$d" $cfg --name golden --trace "$w/t" --tape "$w/u" \
      --measurements "$w/m" >"$w/log" 2>&1; then
    h=$(cat "$w/t" "$w/u" "$w/m" | sha256sum | cut -c1-16)
    n=$(grep -o 'events=[0-9]*' "$w/log")
  else
    h="ERR:$(grep -v '^warning' "$w/log" | head -1 | sha256sum | cut -c1-12)"
    n=""
  fi
  rm -rf "$w"
  echo "$(basename "$d") [$cfg] $h $n"
}
export -f one
{
  for d in "$ROOT"/corpus/*/ "$ROOT"/corpus-fuzz/*/ "$ROOT"/crates/simoxide-sim/tests/models/*/ \
      "$ROOT"/crates/simoxide-cli/bench/models/*/ ${GOLDEN_EXTRA:+"$GOLDEN_EXTRA"/*/}; do
    [ -f "$d/run.json" ] || continue
    for cfg in "--max-measurements 1500 --max-sim-time -1" "--max-measurements -1 --max-sim-time 200" \
        "--seed 7 --max-measurements 4000 --max-sim-time 5000"; do
      printf '%s\0%s\0' "$d" "$cfg"
    done
  done
} | xargs -0 -n 2 -P "${GOLDEN_JOBS:-8}" bash -c 'one "$0" "$1"' | sort
