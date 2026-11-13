#!/usr/bin/env bash
# No-build self-test of tools/agent-cargo: class inference and lock/env
# resolution through --dry-run, plus a real lock-contention check that runs
# no cargo (a fake `cargo` on PATH). Usage: tools/agent-cargo-selftest.sh
set -u
unset STA64_MOLD STA64_LOCK_WAIT STA64_LOCK_DIR RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTC_WRAPPER CARGO_BUILD_JOBS
here=$(cd "$(dirname "$0")" && pwd)
w=$here/agent-cargo
fails=0
tmp=$(mktemp -d "${TMPDIR:-/tmp}/agent-cargo-test.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT

dry() { STA64_LOCK_DIR="$tmp/locks" STA64_SCCACHE=0 "$w" --dry-run "$@" 2>&1; }
expect() { # name, haystack, needle
  case "$2" in *"$3"*) ;; *) echo "FAIL $1: missing '$3' in:"; echo "$2" | sed 's/^/    /'; fails=$((fails+1)) ;; esac
}
refute() {
  case "$2" in *"$3"*) echo "FAIL $1: unexpected '$3' in:"; echo "$2" | sed 's/^/    /'; fails=$((fails+1)) ;; *) ;; esac
}

o=$(dry test -p sta_sim);            expect light-class "$o" "class:  light"
expect jobs-env "$o" "CARGO_BUILD_JOBS=2"; expect cmd-untouched "$o" "cargo test -p sta_sim"
expect env-inc "$o" "CARGO_INCREMENTAL=0"; expect env-dbg "$o" "CARGO_PROFILE_DEV_DEBUG=0"
expect light-lock "$o" "light.lock (else"
o=$(dry check -p sta64);             expect heavy-p "$o" "class:  heavy"
o=$(dry check --package=sta64);      expect heavy-pkg-eq "$o" "class:  heavy"
o=$(dry check -psta64);              expect heavy-psta64 "$o" "class:  heavy"
o=$(dry test -p sta_sim -p sta64);   expect heavy-mixed "$o" "class:  heavy"
o=$(dry check --workspace --all-targets); expect check-class "$o" "class:  check"
expect check-lock "$o" "check.lock"; expect check-target "$o" "target-check"
o=$(dry test --workspace);           expect ws-test-heavy "$o" "class:  heavy"; refute ws-test-target "$o" "target-check"
o=$(dry --light build -p sta64);     expect forced-light "$o" "class:  light"
o=$(dry --heavy test -p sta_sim);    expect forced-heavy "$o" "class:  heavy"; expect heavy-lock "$o" "heavy.lock"
o=$(dry test -p sta_sim -- --jobs x); expect args-after-dashdash "$o" "test -p sta_sim -- --jobs x"
o=$(dry test -j4 -p sta_sim);        refute user-jobs "$o" "CARGO_BUILD_JOBS"
o=$(dry --color always check -p sta64); expect global-opt-heavy "$o" "class:  heavy"
o=$(dry --config net.offline=true check --workspace); expect global-opt-check "$o" "class:  check"
o=$(dry check -p sta64@0.1.0);       expect pkgid-heavy "$o" "class:  heavy"
o=$(STA64_LOCK_WAIT=5 STA64_LOCK_DIR="$tmp/x" STA64_SCCACHE=0 "$w" --dry-run test -p sta_sim 2>&1)
expect lock-dir-override "$o" "$tmp/x/light.lock"; expect lock-wait "$o" "wait 5s"
o=$(STA64_SCCACHE=0 "$w" --dry-run check -p sta_sim 2>&1); refute sccache-off "$o" "RUSTC_WRAPPER"

o=$(dry --heavy check -p sta_sim);  expect heavy2-dry "$o" "heavy2.lock"; expect heavy2-bench-dry "$o" "bench.lock"
o=$(dry --bench ./a2test run hud);   expect bench-class "$o" "class:  bench"; expect bench-lock "$o" "bench.lock (shared)"
expect bench-cmd "$o" "cmd:    ./a2test run hud"; refute bench-no-cargo "$o" "CARGO_INCREMENTAL"

# sccache opt-in by presence, with a fake binary.
mkdir -p "$tmp/bin"; printf '#!/bin/sh\nexit 0\n' > "$tmp/bin/sccache"; chmod +x "$tmp/bin/sccache"
o=$(PATH="$tmp/bin:$PATH" SCCACHE_DIR=/x/y "$w" --dry-run check -p sta_sim 2>&1)
expect sccache-on "$o" "RUSTC_WRAPPER=sccache"; expect sccache-dir "$o" "SCCACHE_DIR=/x/y"; expect sccache-size "$o" "SCCACHE_CACHE_SIZE=20G"
o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 "$w" --dry-run check -p sta_sim 2>&1); refute sccache-disable "$o" "RUSTC_WRAPPER"

# mold is opt-in only.
printf '#!/bin/sh\nexit 0\n' > "$tmp/bin/mold"; chmod +x "$tmp/bin/mold"
o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 "$w" --dry-run check -p sta_sim 2>&1); refute mold-default "$o" "mold"
o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_MOLD=1 "$w" --dry-run check -p sta_sim 2>&1); expect mold-optin "$o" "fuse-ld=mold"

# Lock contention and leaks without cargo: a fake cargo on PATH.
if command -v flock >/dev/null 2>&1; then
  # The fake cargo leaves a background process behind (like a daemon) and
  # exits 3; the wrapper must pass the exit code on and must not leak the lock.
  printf '#!/bin/sh\necho ran-fake-cargo\nsleep 5 >/dev/null 2>&1 &\nexit 3\n' > "$tmp/bin/cargo"; chmod +x "$tmp/bin/cargo"
  mkdir -p "$tmp/locks"
  o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_LOCK_DIR="$tmp/locks" "$w" --heavy check -p sta_sim 2>&1); rc=$?
  expect lock-free-runs "$o" "ran-fake-cargo"
  [ "$rc" = 3 ] || { echo "FAIL exit-code: $rc, want 3"; fails=$((fails+1)); }
  ( exec 8>"$tmp/locks/heavy.lock"; flock -n 8 ) || { echo "FAIL lock-leak (background child kept the lock)"; fails=$((fails+1)); }
  ( exec 8>"$tmp/locks/check.lock"; flock -n 8 && touch "$tmp/held" && sleep 4 ) &
  holder=$!
  n=0; while [ ! -e "$tmp/held" ] && [ "$n" -lt 50 ]; do sleep 0.1; n=$((n+1)); done
  o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_LOCK_WAIT=1 STA64_LOCK_DIR="$tmp/locks" "$w" --check check -p sta_sim 2>&1); rc=$?
  expect lock-busy "$o" "LOCK-BUSY"; [ "$rc" = 75 ] || { echo "FAIL lock-busy: exit $rc, want 75"; fails=$((fails+1)); }
  wait "$holder" 2>/dev/null
  # Second heavy slot: heavy.lock busy and bench.lock free => runs on heavy2.
  rm -f "$tmp/held"
  ( exec 8>"$tmp/locks/heavy.lock"; flock -n 8 && touch "$tmp/held" && sleep 5 ) &
  holder=$!
  n=0; while [ ! -e "$tmp/held" ] && [ "$n" -lt 50 ]; do sleep 0.1; n=$((n+1)); done
  o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_LOCK_WAIT=1 STA64_LOCK_DIR="$tmp/locks" "$w" --heavy check -p sta_sim 2>&1)
  expect heavy2-slot "$o" "ran-fake-cargo"
  # ... but not while a bench session holds bench.lock (taken through --bench).
  PATH="$tmp/bin:$PATH" STA64_LOCK_DIR="$tmp/locks" "$w" --bench sleep 4 &
  bench=$!; sleep 1
  o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_LOCK_WAIT=1 STA64_LOCK_DIR="$tmp/locks" "$w" --heavy check -p sta_sim 2>&1); rc=$?
  expect no-heavy2-during-bench "$o" "LOCK-BUSY"; [ "$rc" = 75 ] || { echo "FAIL no-heavy2-during-bench: exit $rc"; fails=$((fails+1)); }
  wait "$bench" 2>/dev/null; wait "$holder" 2>/dev/null
else
  echo "skip: flock not installed, lock contention checks not run"
fi

[ "$fails" = 0 ] && echo "agent-cargo self-test: ok" || { echo "agent-cargo self-test: $fails failure(s)"; exit 1; }
