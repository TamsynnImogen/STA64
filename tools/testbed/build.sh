#!/usr/bin/env bash
# Build the alpha testbed mods (TOOL-20, docs/ALPHA_TESTBED.md).
#
#   tools/testbed/build.sh <testbed repo> [out dir]
#
# For every mod folder under <testbed repo>/mods: copy it into
# <out>/root/Data/Mods/<mod>, compile maps-src/*.json5 into bzn/, run the load
# check (testbed_check: Native profile, playable races, warnings), then pack
# <out>/dist/<mod>.sta and verify it. The packed archives are checked again
# from an empty root, as testers will install them.
# Child mods (STA64Testbed_<Group>) must resolve to exactly their group's
# switches (tools/testbed/gen_children.py --on).
# Sign with STA64_TESTBED_KEY=<key file> (sta-archive keygen); unsigned otherwise.
set -euo pipefail
here=$(cd "$(dirname "$0")/../.." && pwd)
src=${1:?usage: build.sh <testbed repo> [out dir]}
src=$(cd "$src" && pwd)
out=${2:-$src/build}
cargo=("$here/tools/agent-cargo")
example() { "${cargo[@]}" run -q -p sta_content --example "$@"; }
archive() { "${cargo[@]}" run -q -p sta_vfs --bin sta-archive -- "$@"; }

rm -rf "$out/root" "$out/dist" "$out/check" "$out"/check-*.log
mkdir -p "$out/root/Data/Mods" "$out/dist" "$out/check/Data/Mods"

# The base mod first: child mods need it mounted.
mods=()
for d in "$src"/mods/*/; do
    m=$(basename "$d")
    if grep -qi '^\s*standalone\s*=\s*1' "$d/info.ini"; then mods=("$m" "${mods[@]}"); else mods+=("$m"); fi
done

for m in "${mods[@]}"; do
    d="$src/mods/$m"
    t="$out/root/Data/Mods/$m"
    echo "== $m"
    mkdir -p "$t"
    (cd "$d" && find . -path ./maps-src -prune -o -type f -print0 | cpio -0pdm --quiet "$t")
    if compgen -G "$d/maps-src/*.json5" >/dev/null; then
        example testbed_map -- "$d"/maps-src/*.json5 --out "$t/bzn"
    fi
    example testbed_check -- "$out/root" "$m" --profile native | tee "$out/check-$m.log"
    if [[ $m == STA64Testbed_* ]]; then
        want=$(python3 "$here/tools/testbed/gen_children.py" --on "$m")
        got=$(sed -n 's/^switches on ([0-9]*): //p' "$out/check-$m.log" | tr ' ' '\n' | sort | tr '\n' ' ' | sed 's/ $//')
        [[ "$want" == "$got" ]] || { echo "FAIL $m switches: want [$want] got [$got]"; exit 1; }
        echo "switches match the $m group"
    fi
    archive pack "$t" "$out/dist/$m.sta" --format sta --force
    if [[ -n "${STA64_TESTBED_KEY:-}" ]]; then
        archive sign "$out/dist/$m.sta" --key "$STA64_TESTBED_KEY" --name "STA64 Testbed"
    else
        echo "note: STA64_TESTBED_KEY not set, $m.sta is unsigned"
    fi
    archive verify "$out/dist/$m.sta"
    # Installed layout: Data/Mods/<mod>/<mod>.sta plus the mod's info.ini next to it
    # (the ParentMod chain is read from loose info.ini files; D3 keeps it the tiny ini).
    mkdir -p "$out/check/Data/Mods/$m" && cp "$out/dist/$m.sta" "$d/info.ini" "$out/check/Data/Mods/$m/"
done

echo "== packed archives from an empty root"
for m in "${mods[@]}"; do
    example testbed_check -- "$out/check" "$m" --profile native
done
echo "built: $out/dist"
