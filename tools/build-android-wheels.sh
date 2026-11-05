#!/usr/bin/env bash
# Builds the Python wheels (crates/breader-py) for Android, for apps packaged with Flet (`flet build apk`).
# Usage: tools/build-android-wheels.sh [--python 3.14.10] [--release 20261027] [--abis arm64-v8a,armeabi-v7a,x86_64]
#            [--out target/wheels-android]
#
# Needs ANDROID_NDK_HOME (tested with NDK r27c), maturin, and the Rust targets of the ABIs
# (rustup target add aarch64-linux-android x86_64-linux-android armv7-linux-androideabi).
#
# The wheels are tagged abi3 (cp39-abi3-android_24_<abi>, API level 24), but on Android an extension module
# must link libpython, and PyO3 links the stable-ABI name libpython3.so, which Flet's Android Python does not
# ship. The linker records a library's SONAME, not its file name, so linking against a libpython3.so that
# points to Flet's libpython3.X.so makes the module need libpython3.X.so, which the app does ship. The
# wheels therefore only load on that Python minor version (--python), whatever their tag says.
#
# --python and --release select Flet's python-build distribution (github.com/flet-dev/python-build): use the
# Python version and release of the flet-cli that packages the app (flet_cli.utils.python_versions).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

python_version=3.14.10
release=20261027
abis=arm64-v8a,armeabi-v7a,x86_64
out=target/wheels-android
api=24

while [[ $# -gt 0 ]]; do
  case "$1" in
    --python) python_version="$2"; shift 2 ;;
    --release) release="$2"; shift 2 ;;
    --abis) abis="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the Android NDK}"
toolchain="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
python_minor="${python_version%.*}"
cache=target/android-python/"$release"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

for abi in ${abis//,/ }; do
  case "$abi" in
    arm64-v8a) triple=aarch64-linux-android; clang_triple=aarch64-linux-android ;;
    x86_64) triple=x86_64-linux-android; clang_triple=x86_64-linux-android ;;
    armeabi-v7a) triple=armv7-linux-androideabi; clang_triple=armv7a-linux-androideabi ;;
    *) echo "unsupported ABI: $abi" >&2; exit 2 ;;
  esac

  python_dir="$cache/python-$python_version-$abi"
  if [[ ! -f "$python_dir/libpython$python_minor.so" ]]; then
    mkdir -p "$python_dir"
    curl -fsSL "https://github.com/flet-dev/python-build/releases/download/$release/python-android-dart-$python_version-$abi.tar.gz" \
      | tar -xz -C "$python_dir"
  fi
  link_dir="$cache/link-$python_version-$abi"
  mkdir -p "$link_dir"
  ln -sf "$(cd "$python_dir" && pwd)/libpython$python_minor.so" "$link_dir/libpython3.so"

  target_env="$(echo "$triple" | tr '[:lower:]-' '[:upper:]_')"
  linker="$toolchain/$clang_triple$api-clang"
  env \
    "CARGO_TARGET_${target_env}_LINKER=$linker" \
    "CARGO_TARGET_${target_env}_RUSTFLAGS=-L $(cd "$link_dir" && pwd)" \
    "CC_${triple//-/_}=$linker" \
    "AR_${triple//-/_}=$toolchain/llvm-ar" \
    PYO3_CROSS_PYTHON_VERSION="$python_minor" \
    ANDROID_API_LEVEL="$api" \
    maturin build --release --manifest-path crates/breader-py/Cargo.toml --target "$triple" --out "$out"
done
