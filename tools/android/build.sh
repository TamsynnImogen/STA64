#!/usr/bin/env bash
set -euo pipefail

fail() { printf 'Android build: %s\n' "$*" >&2; exit 1; }
[[ $# -eq 1 && ( $1 == debug || $1 == release ) ]] \
    || fail 'usage: tools/android/build.sh debug|release'
variant=$1
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
project=$repo/packaging/android

"$repo/tools/android/doctor.sh"
sdk=${ANDROID_HOME:-$ANDROID_SDK_ROOT}
ndk=${ANDROID_NDK_HOME:-$ANDROID_NDK_ROOT}
export ANDROID_HOME=$sdk ANDROID_SDK_ROOT=$sdk
export ANDROID_NDK_HOME=$ndk ANDROID_NDK_ROOT=$ndk

toolchain=
for candidate in "$ndk"/toolchains/llvm/prebuilt/*; do
    if [[ -x $candidate/bin/llvm-readelf && -x $candidate/bin/llvm-nm ]]; then
        toolchain=$candidate
        break
    fi
done
[[ -n $toolchain ]] || fail 'NDK LLVM tools not found'
readelf=$toolchain/bin/llvm-readelf
libcxx=$toolchain/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so
system_libs=$ndk/meta/system_libs.json

verify_elf() {
    local file=$1 header alignments alignment
    header=$("$readelf" -h "$file")
    grep -Eq 'Class:[[:space:]]+ELF64' <<<"$header" || fail "$file is not ELF64"
    grep -Eq 'Type:[[:space:]]+DYN' <<<"$header" || fail "$file is not a shared object"
    grep -Eq 'Machine:[[:space:]]+AArch64' <<<"$header" || fail "$file is not AArch64"
    alignments=$("$readelf" --program-headers --wide "$file" | awk '$1 == "LOAD" { print $NF }')
    [[ -n $alignments ]] || fail "$file has no LOAD segments"
    while IFS= read -r alignment; do
        [[ $alignment =~ ^0x[0-9a-fA-F]+$ ]] || fail "$file has unreadable LOAD alignment: $alignment"
        (( alignment >= 0x4000 )) || fail "$file has LOAD alignment $alignment below 16 KiB"
    done <<<"$alignments"
}

verify_needed() {
    local file=$1 bundled=$2 dynamic needed api
    dynamic=$("$readelf" --dynamic --wide "$file")
    while IFS= read -r needed; do
        [[ -n $needed ]] || continue
        [[ $needed =~ ^lib[A-Za-z0-9_+.-]+\.so$ ]] || fail "$file has invalid NEEDED name: $needed"
        if [[ -s $bundled/$needed ]]; then
            continue
        fi
        api=$(awk -F '"' -v name="$needed" '$2 == name { print $4; exit }' "$system_libs")
        [[ $api =~ ^[0-9]+$ && $api -le 26 ]] \
            || fail "$file needs unbundled or API>26 library: $needed"
    done < <(awk '/\(NEEDED\)/ { if (match($0, /\[[^]]+\]/)) print substr($0, RSTART + 1, RLENGTH - 2) }' <<<"$dynamic")
}

verify_activity_exports() {
    local file=$1 symbols
    symbols=$("$readelf" --dyn-syms --wide "$file")
    awk '$NF == "android_main" && $5 == "GLOBAL" && $7 != "UND" { found = 1 } END { exit !found }' \
        <<<"$symbols" || fail "$file does not export android_main"
    awk '$NF == "ANativeActivity_onCreate" && $5 == "GLOBAL" && $7 != "UND" { found = 1 } END { exit !found }' \
        <<<"$symbols" || fail "$file does not export ANativeActivity_onCreate"
}

target_dir=${CARGO_TARGET_DIR:-$repo/target}
if [[ $target_dir != /* ]]; then
    target_dir=$repo/$target_dir
fi
export CARGO_TARGET_DIR=$target_dir
lib=$target_dir/aarch64-linux-android/$variant/libsta64.so
generated=$project/app/build/generated/jniLibs/arm64-v8a

cd "$repo"
renice -n 19 -p $$ >/dev/null || fail 'could not lower build process priority'
export CARGO_INCREMENTAL=0
if [[ $variant == debug ]]; then
    export CARGO_PROFILE_DEV_DEBUG=0
    cargo ndk -t arm64-v8a -P 26 rustc \
        --config 'profile.dev.package."*".opt-level=0' \
        --config 'profile.dev.package.sta_surface.opt-level=0' \
        -p sta64 --lib --crate-type cdylib \
        --locked --jobs 2 -- -C link-arg=-Wl,-z,max-page-size=16384
else
    cargo ndk -t arm64-v8a -P 26 rustc -p sta64 --lib --crate-type cdylib \
        --release --locked --jobs 2 -- -C link-arg=-Wl,-z,max-page-size=16384
fi
[[ -s $lib ]] || fail "native library not produced: $lib"

verify_elf "$lib"
verify_activity_exports "$lib"

mkdir -p "$generated"
cp "$lib" "$generated/libsta64.so"
dynamic=$("$readelf" --dynamic --wide "$lib")
needs_libcxx=false
if grep -Fq 'Shared library: [libc++_shared.so]' <<<"$dynamic"; then
    needs_libcxx=true
    [[ -s $libcxx ]] || fail "required NDK libc++_shared.so is missing: $libcxx"
    cp "$libcxx" "$generated/libc++_shared.so"
else
    rm -f "$generated/libc++_shared.so"
fi
for packaged in "$generated"/*.so; do
    [[ -e $packaged ]] || continue
    case ${packaged##*/} in
        libsta64.so) ;;
        libc++_shared.so) [[ $needs_libcxx == true ]] || fail 'stale libc++_shared.so is packaged' ;;
        *) fail "unexpected packaged native library: $packaged" ;;
    esac
done
verify_elf "$generated/libsta64.so"
verify_activity_exports "$generated/libsta64.so"
if [[ $needs_libcxx == true ]]; then
    verify_elf "$generated/libc++_shared.so"
fi
verify_needed "$generated/libsta64.so" "$generated"
if [[ $needs_libcxx == true ]]; then
    verify_needed "$generated/libc++_shared.so" "$generated"
fi

cd "$project"
if [[ $variant == debug ]]; then
    ./gradlew --no-daemon --max-workers 2 :app:assembleDebug
    apk=$project/app/build/outputs/apk/debug/app-debug.apk
else
    ./gradlew --no-daemon --max-workers 2 :app:assembleRelease
    apk=$project/app/build/outputs/apk/release/app-release-unsigned.apk
fi
[[ -s $apk ]] || fail "APK not produced: $apk"
unzip -tq "$apk" >/dev/null || fail 'APK ZIP structure is invalid'
entries=$(unzip -Z1 "$apk")
grep -Fxq 'lib/arm64-v8a/libsta64.so' <<<"$entries" \
    || fail 'APK has no ARM64 libsta64.so'
if [[ $needs_libcxx == true ]]; then
    grep -Fxq 'lib/arm64-v8a/libc++_shared.so' <<<"$entries" \
        || fail 'APK is missing required ARM64 libc++_shared.so'
elif grep -Fxq 'lib/arm64-v8a/libc++_shared.so' <<<"$entries"; then
    fail 'APK contains an unneeded ARM64 libc++_shared.so'
fi
if awk '/^lib\/arm64-v8a\/.*\.so$/ && $0 != "lib/arm64-v8a/libsta64.so" && $0 != "lib/arm64-v8a/libc++_shared.so" { found = 1 } END { exit !found }' <<<"$entries"; then
    fail 'APK contains an unexpected native library'
fi
if awk '/^lib\// && $0 !~ /^lib\/arm64-v8a\// { found = 1 } END { exit !found }' <<<"$entries"; then
    fail 'APK contains an unexpected ABI'
fi

# Gradle may strip debug symbols while packaging. Verify the actual APK bytes,
# not only the native library staged before Gradle runs.
verified=$project/app/build/verified-apk/$variant/arm64-v8a
mkdir -p "$verified"
unzip -p "$apk" lib/arm64-v8a/libsta64.so > "$verified/libsta64.so" \
    || fail 'could not extract APK libsta64.so'
if [[ $needs_libcxx == true ]]; then
    unzip -p "$apk" lib/arm64-v8a/libc++_shared.so > "$verified/libc++_shared.so" \
        || fail 'could not extract APK libc++_shared.so'
else
    rm -f "$verified/libc++_shared.so"
fi
verify_elf "$verified/libsta64.so"
verify_activity_exports "$verified/libsta64.so"
verify_needed "$verified/libsta64.so" "$verified"
if [[ $needs_libcxx == true ]]; then
    verify_elf "$verified/libc++_shared.so"
    verify_needed "$verified/libc++_shared.so" "$verified"
fi
"$sdk/build-tools/36.0.0/zipalign" -c -P 16 -v 4 "$apk" >/dev/null \
    || fail 'APK native libraries are not 16 KiB ZIP aligned'
apksigner=$sdk/build-tools/36.0.0/apksigner
if [[ $variant == debug ]]; then
    "$apksigner" verify --verbose "$apk" || fail 'debug APK signature verification failed'
else
    if "$apksigner" verify --verbose "$apk"; then
        fail 'release APK unexpectedly has a valid signature; this process must remain unsigned'
    fi
    printf 'Release APK is unsigned; sign it separately with your own key.\n'
fi

printf 'Prepackage native library: %s\nAPK native payload: %s\nAPK: %s\n' \
    "$lib" "$verified/libsta64.so" "$apk"
sha256sum "$lib" "$verified/libsta64.so" "$apk"
if [[ $needs_libcxx == true ]]; then
    sha256sum "$verified/libc++_shared.so"
fi
