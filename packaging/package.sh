#!/usr/bin/env bash
# Package a release build of sta64 for one target (TOOL-09).
#
#   packaging/package.sh <linux|windows|macos> <arch> <binary> <version> <out dir>
#
# linux   -> sta64-<version>-linux-<arch>.tar.gz   (sta64, desktop entry, icon, README)
# windows -> sta64-<version>-windows-<arch>.zip    (sta64.exe with its embedded icon, README)
# macos   -> sta64-<version>-macos-<arch>.zip      (STA64.app: Info.plist, icns, binary)
# Prints the artefact path. Used by .github/workflows/release.yml; runs locally too.
set -euo pipefail
os=${1:?os}; arch=${2:?arch}; bin=${3:?binary}; ver=${4:?version}; out=${5:?out dir}
root=$(cd "$(dirname "$0")/.." && pwd)
name="sta64-$ver-$os-$arch"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$out"
out=$(cd "$out" && pwd)
case "$os" in
linux)
    d="$stage/$name"; mkdir -p "$d"
    cp "$bin" "$d/sta64"; chmod +x "$d/sta64"; strip "$d/sta64" 2>/dev/null || true
    cp "$root/packaging/linux/sta64.desktop" "$d/"
    cp "$root/assets/icons/STA64.png" "$d/sta64.png"
    cp "$root/README.md" "$d/"
    tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
    echo "$out/$name.tar.gz" ;;
windows)
    d="$stage/$name"; mkdir -p "$d"
    cp "$bin" "$d/sta64.exe"
    cp "$root/README.md" "$d/"
    (cd "$stage" && 7z a -tzip -bso0 "$out/$name.zip" "$name")
    echo "$out/$name.zip" ;;
macos)
    app="$stage/STA64.app"; mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$bin" "$app/Contents/MacOS/sta64"; chmod +x "$app/Contents/MacOS/sta64"
    sed "s/@VERSION@/$ver/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
    iconset="$stage/STA64.iconset"; mkdir -p "$iconset"
    for s in 16 32 128 256 512; do
        sips -z $s $s "$root/assets/icons/STA64.png" --out "$iconset/icon_${s}x${s}.png" >/dev/null
        sips -z $((s * 2)) $((s * 2)) "$root/assets/icons/STA64.png" --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns "$iconset" -o "$app/Contents/Resources/STA64.icns"
    cp "$root/README.md" "$stage/"
    (cd "$stage" && ditto -c -k --keepParent STA64.app "$out/$name.zip")
    echo "$out/$name.zip" ;;
*) echo "unknown os $os" >&2; exit 2 ;;
esac
