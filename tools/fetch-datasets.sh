#!/usr/bin/env bash
# Usage: tools/fetch-datasets.sh [dynamsoft|zxing-samples|all]   (default: all)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
which="${1:-all}"

fetch_dynamsoft() {
    local dest="$root/data/dynamsoft"
    local url="https://github.com/Dynamsoft/datasets-from-dynamsoft"

    if [ -d "$dest/.git" ]; then
        git -C "$dest" pull --ff-only
    else
        mkdir -p "$(dirname "$dest")"
        git clone --depth 1 "$url" "$dest"
    fi
}

# zxing-cpp test images and expected results, used as test data only. Sparse checkout of
# test/samples at a pinned release; annotations.json files are (re)generated on every run.
zxing_tag="v3.1.1"

fetch_zxing_samples() {
    local dest="$root/data/zxing-samples"
    local url="https://github.com/zxing-cpp/zxing-cpp"

    if [ ! -d "$dest/.git" ]; then
        mkdir -p "$(dirname "$dest")"
        git clone --depth 1 --filter=blob:none --sparse --branch "$zxing_tag" "$url" "$dest"
        git -C "$dest" sparse-checkout set test/samples
    fi
    (cd "$root" && cargo build --release -p bench --bin zxing-samples-gt)
    "$root/target/release/zxing-samples-gt" "$dest/test/samples"
}

case "$which" in
    dynamsoft) fetch_dynamsoft ;;
    zxing-samples) fetch_zxing_samples ;;
    all) fetch_dynamsoft; fetch_zxing_samples ;;
    *) echo "usage: $0 [dynamsoft|zxing-samples|all]" >&2; exit 2 ;;
esac
