#!/usr/bin/env bash
# No-build self-test of tools/agent-cargo: class inference and lock/env
# resolution through --dry-run, plus a real lock-contention check that runs
# no cargo (a fake `cargo` on PATH). Usage: tools/agent-cargo-selftest.sh
set -u
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
expect jobs-after-sub "$o" "cargo test --jobs 2 -p sta_sim"
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
o=$(dry test -p sta_sim -- --jobs x); expect jobs-before-dashdash "$o" "test --jobs 2 -p sta_sim -- --jobs x"
o=$(dry test -j4 -p sta_sim);        refute user-jobs "$o" "--jobs 2"
o=$(dry fmt -p sta_sim);             refute fmt-no-jobs "$o" "--jobs"
o=$(STA64_LOCK_WAIT=5 STA64_LOCK_DIR="$tmp/x" STA64_SCCACHE=0 "$w" --dry-run test -p sta_sim 2>&1)
expect lock-dir-override "$o" "$tmp/x/light.lock"; expect lock-wait "$o" "wait 5s"
o=$(STA64_SCCACHE=0 "$w" --dry-run check -p sta_sim 2>&1); refute sccache-off "$o" "RUSTC_WRAPPER"

# sccache opt-in by presence, with a fake binary.
mkdir -p "$tmp/bin"; printf '#!/bin/sh\nexit 0\n' > "$tmp/bin/sccache"; chmod +x "$tmp/bin/sccache"
o=$(PATH="$tmp/bin:$PATH" SCCACHE_DIR=/x/y "$w" --dry-run check -p sta_sim 2>&1)
expect sccache-on "$o" "RUSTC_WRAPPER=sccache"; expect sccache-dir "$o" "SCCACHE_DIR=/x/y"; expect sccache-size "$o" "SCCACHE_CACHE_SIZE=20G"
o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 "$w" --dry-run check -p sta_sim 2>&1); refute sccache-disable "$o" "RUSTC_WRAPPER"

# mold is opt-in only.
printf '#!/bin/sh\nexit 0\n' > "$tmp/bin/mold"; chmod +x "$tmp/bin/mold"
o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 "$w" --dry-run check -p sta_sim 2>&1); refute mold-default "$o" "mold"
o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_MOLD=1 "$w" --dry-run check -p sta_sim 2>&1); expect mold-optin "$o" "fuse-ld=mold"

# Lock contention without cargo: a held lock makes the wrapper exit 75.
if command -v flock >/dev/null 2>&1; then
  printf '#!/bin/sh\necho ran-fake-cargo\n' > "$tmp/bin/cargo"; chmod +x "$tmp/bin/cargo"
  mkdir -p "$tmp/locks"
  o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_LOCK_DIR="$tmp/locks" "$w" check -p sta_sim 2>&1)
  expect lock-free-runs "$o" "ran-fake-cargo"
  ( exec 8>"$tmp/locks/heavy.lock"; flock -n 8 && sleep 4 ) &
  holder=$!; sleep 1
  o=$(PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_LOCK_WAIT=1 STA64_LOCK_DIR="$tmp/locks" "$w" --heavy check -p sta_sim 2>&1); rc=$?
  expect lock-busy "$o" "LOCK-BUSY"; [ "$rc" = 75 ] || { echo "FAIL lock-busy: exit $rc, want 75"; fails=$((fails+1)); }
  wait "$holder" 2>/dev/null
  # The lock must be free again once the wrapper's cargo has exited.
  PATH="$tmp/bin:$PATH" STA64_SCCACHE=0 STA64_LOCK_DIR="$tmp/locks" "$w" --heavy check -p sta_sim >/dev/null 2>&1
  ( exec 8>"$tmp/locks/heavy.lock"; flock -n 8 ) || { echo "FAIL lock-leak"; fails=$((fails+1)); }
else
  echo "skip: flock not installed, lock contention checks not run"
fi

[ "$fails" = 0 ] && echo "agent-cargo self-test: ok" || { echo "agent-cargo self-test: $fails failure(s)"; exit 1; }
