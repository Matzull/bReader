#!/usr/bin/env bash
# Runs the regression gate against the versioned baselines in tools/bench/baselines.
# Usage: tools/check-regression.sh [--dynamsoft] [--zxing-samples] [--barber] [--barber-full] [--all]
#   Synthetic sets (data/synth seed 1, data/synth-2024 seed 2024) are generated when missing.
#   --dynamsoft also gates data/dynamsoft/challenging-images, skewed-datamatrix, UPS-Synthetic-Labels,
#   pdf417-small-module-size and the videos (needs tools/fetch-datasets.sh and ffmpeg).
#   --zxing-samples also gates data/zxing-samples/test/samples (needs tools/fetch-datasets.sh).
#   --barber gates a deterministic 1/8 subset of data/barber (BarBeR, ~1100 of 8748 images; needs
#   data/barber/BarBeR - Dataset, see docs/datasets.md) against barber-subset-*.json.
#   --barber-full gates the whole BarBeR set against barber-*.json (about 10 minutes per effort).
#   --all is --dynamsoft, --zxing-samples and --barber-full.
#   A BarBeR stage is skipped with a message when data/barber is missing.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
base=tools/bench/baselines
bin=target/release
dynamsoft=0
zxing=0
barber=none
for arg in "$@"; do
    case "$arg" in
        --dynamsoft) dynamsoft=1 ;;
        --zxing-samples) zxing=1 ;;
        --barber) [ "$barber" = full ] || barber=subset ;;
        --barber-full) barber=full ;;
        --all) dynamsoft=1; zxing=1; barber=full ;;
        *) echo "unknown option $arg" >&2; exit 2 ;;
    esac
done

cargo build --release -p bench -p video-bench -p synth

[ -f data/synth/clean/annotations.json ] || "$bin/synth" data/synth --seed 1
[ -f data/synth-2024/clean/annotations.json ] || "$bin/synth" data/synth-2024 --count 60 --seed 2024

if [ "$zxing" = 1 ] && [ ! -f data/zxing-samples/test/samples/qrcode-1/annotations.json ]; then
    echo "data/zxing-samples is missing or has no annotations: run tools/fetch-datasets.sh" >&2
    exit 2
fi

if [ "$barber" != none ] && [ ! -f "data/barber/BarBeR - Dataset/Annotations/VIA/Artelab.json" ]; then
    echo "data/barber is missing or incomplete (see docs/datasets.md#barber): skipping the BarBeR stage"
    barber=none
fi

status=0
gate() {
    if ! "$@"; then status=1; fi
}
for effort in fast balanced exhaustive; do
    echo "=== synth seed 1, $effort"
    gate "$bin/bench" data/synth --effort "$effort" --check "$base/synth-seed1-$effort.json"
    echo "=== synth seed 2024, $effort"
    gate "$bin/bench" data/synth-2024 --effort "$effort" --check "$base/synth-seed2024-$effort.json"
    if [ "$dynamsoft" = 1 ]; then
        echo "=== dynamsoft challenging-images, $effort"
        gate "$bin/bench" data/dynamsoft/challenging-images --effort "$effort" \
            --check "$base/dynamsoft-challenging-$effort.json"
        echo "=== dynamsoft skewed-datamatrix, $effort"
        gate "$bin/bench" data/dynamsoft/skewed-datamatrix --effort "$effort" \
            --check "$base/dynamsoft-skewed-$effort.json"
        echo "=== dynamsoft UPS-Synthetic-Labels, $effort"
        gate "$bin/bench" data/dynamsoft/UPS-Synthetic-Labels --effort "$effort" \
            --check "$base/dynamsoft-ups-$effort.json"
        echo "=== dynamsoft pdf417-small-module-size, $effort"
        gate "$bin/bench" data/dynamsoft/pdf417-small-module-size --effort "$effort" \
            --check "$base/dynamsoft-pdf417-$effort.json"
    fi
    if [ "$barber" = subset ]; then
        echo "=== BarBeR 1/8 subset, $effort"
        gate "$bin/bench" data/barber --subset 8 --effort "$effort" --check "$base/barber-subset-$effort.json"
    elif [ "$barber" = full ]; then
        echo "=== BarBeR, $effort"
        gate "$bin/bench" data/barber --effort "$effort" --check "$base/barber-$effort.json"
    fi
    if [ "$zxing" = 1 ]; then
        echo "=== zxing-cpp samples, $effort"
        gate "$bin/bench" data/zxing-samples/test/samples --effort "$effort" \
            --check "$base/zxing-samples-$effort.json"
    fi
done
if [ "$dynamsoft" = 1 ]; then
    echo "=== dynamsoft videos"
    gate "$bin/video-bench" data/dynamsoft/video-based-testing --check "$base/dynamsoft-video.json"
fi
exit "$status"
