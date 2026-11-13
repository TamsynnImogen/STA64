#!/usr/bin/env bash
# No-build self-test of tools/agent-cargo: class inference and lock/env
# resolution through --dry-run, plus a real lock-contention check that runs
# no cargo (a fake `cargo` on PATH). Usage: tools/agent-cargo-selftest.sh
set -u
unset STA64_QUEUE STA64_QUEUE_KEY STA64_QUEUE_ACTIVE STA64_QUEUE_STALE STA64_MOLD STA64_LOCK_WAIT STA64_LOCK_DIR RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTC_WRAPPER CARGO_BUILD_JOBS
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

o=$(dry --heavy check -p sta_sim);  expect queue-dry "$o" "queue:  $tmp/locks/queue/heavy"; expect heavy2-dry "$o" "heavy2.lock"; expect heavy2-bench-dry "$o" "bench.lock"
o=$(STA64_QUEUE=0 dry --heavy check -p x); expect queue-off "$o" "queue:  off"
o=$(dry --no-queue --heavy check -p x); expect no-queue-flag "$o" "queue:  off"
o=$(STA64_QUEUE=0 dry --queue --heavy check -p x); expect queue-flag "$o" "queue:  $tmp/locks/queue/heavy"
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

  # ---- FIFO queue. Each waiter is a "worktree" (STA64_QUEUE_KEY); the fake
  # cargo logs "<key> start|end <ns>" and sleeps FAKE_SLEEP seconds.
  mkdir -p "$tmp/fifobin"
  cat > "$tmp/fifobin/cargo" <<'EOS'
#!/bin/sh
echo "$STA64_QUEUE_KEY start $(date +%s%N)" >> "$FIFO_LOG"
sleep "${FAKE_SLEEP:-1}"
echo "$STA64_QUEUE_KEY end $(date +%s%N)" >> "$FIFO_LOG"
EOS
  chmod +x "$tmp/fifobin/cargo"
  # fq <key> <log> <extra env...> -- <flags>: run a queued fake build
  fq() { local key=$1 log=$2; shift 2
    env PATH="$tmp/fifobin:$PATH" STA64_SCCACHE=0 STA64_LOCK_DIR="$tmp/fl" STA64_QUEUE_KEY="$key" FIFO_LOG="$log" "$@" ; }
  order() { awk '$2=="start"{printf "%s ", $1}' "$1"; }
  maxconc() { awk '{ if ($2=="start") c++; else c--; if (c>m) m=c } END { print m+0 }' "$1"; }
  fresh() { rm -rf "$tmp/fl"; mkdir -p "$tmp/fl"; : > "$1"; }

  # 1. FIFO among waiters behind a holder (check.lock: one slot).
  L=$tmp/log1; fresh "$L"
  fq h "$L" FAKE_SLEEP=3 "$w" --check check -p x >/dev/null 2>&1 & p0=$!
  sleep 0.5
  for k in a b c; do fq $k "$L" FAKE_SLEEP=1 STA64_LOCK_WAIT=60 "$w" --check check -p x >/dev/null 2>&1 & sleep 0.4; done
  wait
  o=$(order "$L"); [ "$o" = "h a b c " ] || { echo "FAIL fifo-order: '$o'"; fails=$((fails+1)); }
  [ -z "$(ls -A "$tmp/fl/queue/check" 2>/dev/null)" ] || { echo "FAIL tickets-removed: $(ls "$tmp/fl/queue/check")"; fails=$((fails+1)); }

  # 2. A waiter that got LOCK-BUSY and retries keeps its place over a later arrival.
  L=$tmp/log2; fresh "$L"
  fq h "$L" FAKE_SLEEP=6 "$w" --check check -p x >/dev/null 2>&1 & sleep 0.5
  o=$(fq w1 "$L" STA64_LOCK_WAIT=2 "$w" --check check -p x 2>&1); rc=$?
  [ "$rc" = 75 ] || { echo "FAIL retry-first-busy: exit $rc"; fails=$((fails+1)); }
  fq w2 "$L" FAKE_SLEEP=1 STA64_LOCK_WAIT=60 "$w" --check check -p x >/dev/null 2>&1 & sleep 0.4
  fq w1 "$L" FAKE_SLEEP=1 STA64_LOCK_WAIT=60 "$w" --check check -p x >/dev/null 2>&1 &
  wait
  o=$(order "$L"); [ "$o" = "h w1 w2 " ] || { echo "FAIL retry-keeps-place: '$o'"; fails=$((fails+1)); }

  # 3. A dead head ticket (idle past STA64_QUEUE_ACTIVE) does not block; a
  #    long-stale one is deleted.
  L=$tmp/log3; fresh "$L"; mkdir -p "$tmp/fl/queue/check"
  : > "$tmp/fl/queue/check/0000000000000000001-dead"; touch -d '60 seconds ago' "$tmp/fl/queue/check/0000000000000000001-dead"
  : > "$tmp/fl/queue/check/0000000000000000002-ghost"; touch -d '3 hours ago' "$tmp/fl/queue/check/0000000000000000002-ghost"
  : > "$tmp/fl/queue/check/0000000000000000003-live"; touch -d '3 seconds ago' "$tmp/fl/queue/check/0000000000000000003-live"
  o=$(fq a "$L" FAKE_SLEEP=0 STA64_LOCK_WAIT=3 "$w" --check check -p x 2>&1); rc=$?
  [ "$rc" = 75 ] || { echo "FAIL live-head-blocks: exit $rc ($o)"; fails=$((fails+1)); }   # an active older ticket does hold us back
  [ ! -e "$tmp/fl/queue/check/0000000000000000002-ghost" ] || { echo "FAIL stale-deleted"; fails=$((fails+1)); }
  rm -f "$tmp/fl/queue/check/0000000000000000003-live"
  o=$(fq a "$L" FAKE_SLEEP=0 STA64_LOCK_WAIT=3 "$w" --check check -p x 2>&1); rc=$?
  [ "$rc" = 0 ] && [ "$(order "$L")" = "a " ] || { echo "FAIL dead-head-blocks: exit $rc, '$(order "$L")'"; fails=$((fails+1)); }

  # 4. Two heavy slots serve the two oldest; the third waits for one to finish.
  L=$tmp/log4; fresh "$L"
  for k in a b c; do fq $k "$L" FAKE_SLEEP=3 STA64_LOCK_WAIT=60 "$w" --heavy check -p x >/dev/null 2>&1 & sleep 0.4; done
  wait
  o=$(order "$L"); [ "$o" = "a b c " ] || { echo "FAIL heavy2-order: '$o'"; fails=$((fails+1)); }
  m=$(maxconc "$L"); [ "$m" = 2 ] || { echo "FAIL heavy2-concurrency: $m, want 2"; fails=$((fails+1)); }
  # 5. bench.lock held: one heavy slot only, still in order.
  L=$tmp/log5; fresh "$L"
  PATH="$tmp/fifobin:$PATH" STA64_LOCK_DIR="$tmp/fl" "$w" --bench sleep 8 & bench=$!; sleep 0.5
  for k in a b; do fq $k "$L" FAKE_SLEEP=2 STA64_LOCK_WAIT=60 "$w" --heavy check -p x >/dev/null 2>&1 & sleep 0.4; done
  wait "$bench"; wait
  o=$(order "$L"); [ "$o" = "a b " ] || { echo "FAIL bench-order: '$o'"; fails=$((fails+1)); }
  m=$(maxconc "$L"); [ "$m" = 1 ] || { echo "FAIL bench-one-slot: $m, want 1"; fails=$((fails+1)); }
  # 6. STA64_QUEUE=0 leaves no tickets.
  L=$tmp/log6; fresh "$L"
  fq a "$L" FAKE_SLEEP=0 STA64_QUEUE=0 "$w" --check check -p x >/dev/null 2>&1
  [ ! -e "$tmp/fl/queue" ] || { echo "FAIL queue-off-no-dir"; fails=$((fails+1)); }
else
  echo "skip: flock not installed, lock contention checks not run"
fi

[ "$fails" = 0 ] && echo "agent-cargo self-test: ok" || { echo "agent-cargo self-test: $fails failure(s)"; exit 1; }
