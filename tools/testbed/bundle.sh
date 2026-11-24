#!/usr/bin/env bash
# Package the closed alpha (TOOL-25, docs/ALPHA_TESTBED.md).
#
#   tools/testbed/bundle.sh <testbed repo> <sta64 binary> <version> [out dir]
#
# Needs a finished tools/testbed/build.sh run (its build/check tree is the
# installed layout: Data/Mods/<Mod>/<Mod>.sta + info.ini). Writes
# <out>/STA64-Testbed-<version>-<os>-<arch>/ and a .zip of it, with the game
# binary, the eight mod folders, a launcher, the tester README, CREDITS.md and
# SHA256SUMS. The OS is taken from the binary (.exe = windows).
set -euo pipefail
here=$(cd "$(dirname "$0")/../.." && pwd)
src=$(cd "${1:?usage: bundle.sh <testbed repo> <sta64 binary> <version> [out dir]}" && pwd)
bin=${2:?sta64 binary}
ver=${3:?version}
out=${4:-$src/dist}
case "$bin" in *.exe) os=windows ;; *) os=linux ;; esac
arch=x86_64
name="STA64-Testbed-$ver-$os-$arch"
dir="$out/$name"
check="$src/build/check"
[[ -d "$check/Data/Mods/STA64Testbed" ]] || { echo "run tools/testbed/build.sh first"; exit 1; }

rm -rf "$dir" "$out/$name.zip"
mkdir -p "$dir"
cp -r "$check/Data" "$dir/"
cp "$bin" "$dir/"
cp "$here/tools/testbed/README-TESTERS.md" "$dir/README.md"
cp "$src/CREDITS.md" "$dir/"
if [[ $os == linux ]]; then
    chmod +x "$dir/$(basename "$bin")"
    strip "$dir/$(basename "$bin")" 2>/dev/null || true
    cat > "$dir/run-testbed.sh" <<'RUN'
#!/usr/bin/env sh
# ./run-testbed.sh [Group]   Group: Combat Defence Behaviour Economy Production Fleet Sandbox (default)
here=$(cd "$(dirname "$0")" && pwd)
group=${1:-Sandbox}
exec "$here/sta64" --install-root "$here" --mod "STA64Testbed_$group" --nointro
RUN
    chmod +x "$dir/run-testbed.sh"
else
    cat > "$dir/run-testbed.bat" <<'RUN'
@echo off
rem run-testbed.bat [Group]   Group: Combat Defence Behaviour Economy Production Fleet Sandbox (default)
set GROUP=%1
if "%GROUP%"=="" set GROUP=Sandbox
"%~dp0sta64.exe" --install-root "%~dp0." --mod STA64Testbed_%GROUP% --nointro
RUN
fi
(cd "$dir" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
(cd "$out" && zip -qr "$name.zip" "$name")
echo "bundle: $out/$name.zip ($(du -h "$out/$name.zip" | cut -f1))"
