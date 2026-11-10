#!/usr/bin/env bash
set -euo pipefail

fail() { printf 'Android doctor: %s\n' "$*" >&2; exit 1; }

sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
[[ -n $sdk && -d $sdk ]] || fail 'set ANDROID_HOME or ANDROID_SDK_ROOT to an installed SDK'
[[ -n $ndk && -d $ndk ]] || fail 'set ANDROID_NDK_HOME or ANDROID_NDK_ROOT to NDK 29.0.14206865'
if [[ -n ${ANDROID_HOME:-} && -n ${ANDROID_SDK_ROOT:-} && $ANDROID_HOME != "$ANDROID_SDK_ROOT" ]]; then
    fail 'ANDROID_HOME and ANDROID_SDK_ROOT disagree'
fi
if [[ -n ${ANDROID_NDK_HOME:-} && -n ${ANDROID_NDK_ROOT:-} && $ANDROID_NDK_HOME != "$ANDROID_NDK_ROOT" ]]; then
    fail 'ANDROID_NDK_HOME and ANDROID_NDK_ROOT disagree'
fi
[[ -f $sdk/platforms/android-36/android.jar ]] || fail 'SDK platform android-36 is missing'
[[ -x $sdk/build-tools/36.0.0/apksigner ]] || fail 'SDK build-tools 36.0.0/apksigner is missing'
[[ -x $sdk/build-tools/36.0.0/zipalign ]] || fail 'SDK build-tools 36.0.0/zipalign is missing'
[[ -f $ndk/source.properties ]] || fail 'NDK source.properties is missing'
[[ -f $ndk/meta/system_libs.json ]] || fail 'NDK public system library metadata is missing'
grep -Eq '^Pkg.Revision[[:space:]]*=[[:space:]]*29\.0\.14206865[[:space:]]*$' "$ndk/source.properties" \
    || fail 'NDK must be 29.0.14206865'

toolchain=
for candidate in "$ndk"/toolchains/llvm/prebuilt/*; do
    if [[ -x $candidate/bin/llvm-readelf && -x $candidate/bin/llvm-nm && -x $candidate/bin/aarch64-linux-android26-clang ]]; then
        toolchain=$candidate
        break
    fi
done
[[ -n $toolchain ]] || fail 'NDK 29 ARM64 LLVM toolchain is missing'

java_home=${JAVA_HOME:-}
[[ -n $java_home && -x $java_home/bin/java && -x $java_home/bin/javac ]] \
    || fail 'set JAVA_HOME to JDK 21'
java_line=$("$java_home/bin/java" -version 2>&1 | head -n 1)
[[ $java_line =~ version\ \"21([.\+\"]|$) ]] || fail "JDK 21 required; found: $java_line"
command -v cargo >/dev/null || fail 'cargo is missing'
command -v rustup >/dev/null || fail 'rustup is missing'
command -v unzip >/dev/null || fail 'unzip is missing'
command -v sha256sum >/dev/null || fail 'sha256sum is missing'
rustup target list --installed --toolchain 1.97.1 | grep -Fxq aarch64-linux-android \
    || fail 'install the aarch64-linux-android target for Rust 1.97.1'
ndk_version=$(cargo ndk --version 2>&1) || fail 'cargo-ndk 4.1.2 is missing'
[[ $ndk_version == *4.1.2* ]] || fail "cargo-ndk 4.1.2 required; found: $ndk_version"

printf 'SDK: %s\nNDK: %s\nLLVM: %s\nJava: %s\ncargo-ndk: %s\n' \
    "$sdk" "$ndk" "$toolchain" "$java_line" "$ndk_version"
