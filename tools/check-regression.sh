#!/usr/bin/env bash
# Runs the regression gate against the versioned baselines in tools/bench/baselines.
# Usage: tools/check-regression.sh [--dynamsoft]
#   Synthetic sets (data/synth seed 1, data/synth-2024 seed 2024) are generated when missing.
#   --dynamsoft also gates data/dynamsoft/challenging-images and the videos (needs
#   tools/fetch-datasets.sh and ffmpeg).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
base=tools/bench/baselines
bin=target/release
dynamsoft=0
[ "${1:-}" = "--dynamsoft" ] && dynamsoft=1

cargo build --release -p bench -p video-bench -p synth

[ -f data/synth/clean/annotations.json ] || "$bin/synth" data/synth --seed 1
[ -f data/synth-2024/clean/annotations.json ] || "$bin/synth" data/synth-2024 --count 60 --seed 2024

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
    fi
done
if [ "$dynamsoft" = 1 ]; then
    echo "=== dynamsoft videos"
    gate "$bin/video-bench" data/dynamsoft/video-based-testing --check "$base/dynamsoft-video.json"
fi
exit "$status"
