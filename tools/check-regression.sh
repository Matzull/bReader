#!/usr/bin/env bash
# Runs the regression gate against the versioned baselines in tools/bench/baselines.
# Usage: tools/check-regression.sh [--tier a|b|c1|c2] [--efforts fast,balanced,exhaustive]
#            [--synthetic-only] [--dynamsoft] [--zxing-samples] [--barber] [--barber-full] [--all]
#
# Tiers (docs/testing.md, docs/TEST-STUDY.md); the default is tier a:
#   a   every change, about 35 s of gates. Balanced on synth seed 1, synth seed 2024,
#       Dynamsoft challenging-images, Dynamsoft pdf417-small-module-size and zxing-cpp samples.
#   b   before merging, about 10 minutes. Everything in a, plus all three efforts on synth seed 2024
#       and every cheap real set (skewed-datamatrix and UPS-Synthetic-Labels too), the BarBeR 1/8
#       subset at Balanced and Exhaustive, and the video gate with the tracker engine only.
#   c1  changes that raise false-positive exposure, about 35 minutes. Like b, but the full BarBeR set
#       at Balanced and Exhaustive instead of the subset.
#   c2  phase end or release, about 42 minutes. Every set at every effort, the full BarBeR set and the
#       video gate with all engines. --all is an alias of c2.
#   Synth seed 1 is gated at Balanced only in every tier. Any false positive fails the run.
#
# --efforts E1,E2 keeps only those efforts of the selected tier (fast, balanced, exhaustive); the video
# gate is not affected.
# --synthetic-only runs only the synthetic sets of the tier (CI: the real datasets are not fetched).
#
# Without --tier, the older per-set flags select what runs, always at all three efforts (or --efforts):
#   Synthetic sets (data/synth seed 1, data/synth-2024 seed 2024) are generated when missing.
#   --dynamsoft also gates data/dynamsoft/challenging-images, skewed-datamatrix, UPS-Synthetic-Labels,
#   pdf417-small-module-size and the videos, all engines (needs tools/fetch-datasets.sh and ffmpeg).
#   --zxing-samples also gates data/zxing-samples/test/samples (needs tools/fetch-datasets.sh).
#   --barber gates a deterministic 1/8 subset of data/barber (BarBeR, ~1100 of 8748 images; needs
#   data/barber/BarBeR - Dataset, see docs/datasets.md) against barber-subset-*.json.
#   --barber-full gates the whole BarBeR set against barber-*.json (5 to 18 minutes per effort).
#   --all is --tier c2.
#   A BarBeR stage is skipped with a message when data/barber is missing.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
base=tools/bench/baselines
bin=target/release
all="fast balanced exhaustive"

# Efforts per gate set (space separated; empty = not run) and the video engines (empty = no video gate).
e_synth1="" e_synth2024="" e_chal="" e_skew="" e_ups="" e_pdf="" e_zx="" e_barsub="" e_barfull=""
video=""

tier=""
legacy=0
efforts_filter=""
synthetic_only=0
want_dynamsoft=0 want_zxing=0 want_barber=none

set_tier() {
    case "$1" in
        a)
            e_synth1=balanced e_synth2024=balanced e_chal=balanced e_pdf=balanced e_zx=balanced
            ;;
        b)
            e_synth1=balanced e_synth2024=$all e_chal=$all e_skew=$all e_ups=$all e_pdf=$all e_zx=$all
            e_barsub="balanced exhaustive" video=tracker
            ;;
        c1)
            e_synth1=balanced e_synth2024=$all e_chal=$all e_skew=$all e_ups=$all e_pdf=$all e_zx=$all
            e_barfull="balanced exhaustive" video=tracker
            ;;
        c2)
            e_synth1=balanced e_synth2024=$all e_chal=$all e_skew=$all e_ups=$all e_pdf=$all e_zx=$all
            e_barfull=$all video=tracker,fast,balanced
            ;;
        *) echo "unknown tier $1 (a, b, c1 or c2)" >&2; exit 2 ;;
    esac
}

while [ $# -gt 0 ]; do
    arg="$1"
    shift
    case "$arg" in
        --tier) [ $# -gt 0 ] || { echo "--tier needs a value" >&2; exit 2; }; tier="$1"; shift ;;
        --tier=*) tier="${arg#--tier=}" ;;
        --efforts) [ $# -gt 0 ] || { echo "--efforts needs a value" >&2; exit 2; }; efforts_filter="$1"; shift ;;
        --efforts=*) efforts_filter="${arg#--efforts=}" ;;
        --synthetic-only) synthetic_only=1 ;;
        --dynamsoft) legacy=1; want_dynamsoft=1 ;;
        --zxing-samples) legacy=1; want_zxing=1 ;;
        --barber) legacy=1; [ "$want_barber" = full ] || want_barber=subset ;;
        --barber-full) legacy=1; want_barber=full ;;
        --all) tier=c2 ;;
        *) echo "unknown option $arg" >&2; exit 2 ;;
    esac
done

if [ -n "$tier" ]; then
    set_tier "$(echo "$tier" | tr 'A-Z' 'a-z')"
elif [ "$legacy" = 1 ]; then
    # Per-set flags: the synthetic sets plus the selected ones at every effort.
    e_synth1=balanced e_synth2024=$all
    if [ "$want_dynamsoft" = 1 ]; then
        e_chal=$all e_skew=$all e_ups=$all e_pdf=$all video=tracker,fast,balanced
    fi
    [ "$want_zxing" = 1 ] && e_zx=$all
    [ "$want_barber" = subset ] && e_barsub=$all
    [ "$want_barber" = full ] && e_barfull=$all
else
    set_tier a
fi

if [ "$synthetic_only" = 1 ]; then
    e_chal="" e_skew="" e_ups="" e_pdf="" e_zx="" e_barsub="" e_barfull="" video=""
fi

# --efforts keeps only the listed efforts of every set.
if [ -n "$efforts_filter" ]; then
    wanted=" $(echo "$efforts_filter" | tr ',' ' ') "
    for w in $wanted; do
        case "$w" in fast | balanced | exhaustive) ;; *) echo "unknown effort $w" >&2; exit 2 ;; esac
    done
    keep() {
        local out="" e
        for e in $1; do
            case "$wanted" in *" $e "*) out="$out $e" ;; esac
        done
        echo "$out"
    }
    e_synth1=$(keep "$e_synth1") e_synth2024=$(keep "$e_synth2024") e_chal=$(keep "$e_chal")
    e_skew=$(keep "$e_skew") e_ups=$(keep "$e_ups") e_pdf=$(keep "$e_pdf") e_zx=$(keep "$e_zx")
    e_barsub=$(keep "$e_barsub") e_barfull=$(keep "$e_barfull")
fi

cargo build --release -p bench -p video-bench -p synth

[ -f data/synth/clean/annotations.json ] || "$bin/synth" data/synth --seed 1
[ -f data/synth-2024/clean/annotations.json ] || "$bin/synth" data/synth-2024 --count 60 --seed 2024

need_data() {
    if [ ! -d "$1" ]; then
        echo "$1 is missing: run tools/fetch-datasets.sh (or use --synthetic-only)" >&2
        exit 2
    fi
}
[ -z "$e_chal$e_skew$e_ups$e_pdf$video" ] || need_data data/dynamsoft
[ -z "$e_zx" ] || [ -f data/zxing-samples/test/samples/qrcode-1/annotations.json ] || {
    echo "data/zxing-samples is missing or has no annotations: run tools/fetch-datasets.sh" >&2
    exit 2
}

if [ -n "$e_barsub$e_barfull" ] && [ ! -f "data/barber/BarBeR - Dataset/Annotations/VIA/Artelab.json" ]; then
    echo "data/barber is missing or incomplete (see docs/datasets.md#barber): skipping the BarBeR stage"
    e_barsub="" e_barfull=""
fi

status=0
gate() {
    if ! "$@"; then status=1; fi
}
# bench_gate <efforts> <label> <baseline stem> <bench args...>
bench_gate() {
    local efforts="$1" label="$2" stem="$3" effort
    shift 3
    for effort in $efforts; do
        echo "=== $label, $effort"
        gate "$bin/bench" "$@" --effort "$effort" --check "$base/$stem-$effort.json"
    done
}
bench_gate "$e_synth1" "synth seed 1" synth-seed1 data/synth
bench_gate "$e_synth2024" "synth seed 2024" synth-seed2024 data/synth-2024
bench_gate "$e_chal" "dynamsoft challenging-images" dynamsoft-challenging data/dynamsoft/challenging-images
bench_gate "$e_skew" "dynamsoft skewed-datamatrix" dynamsoft-skewed data/dynamsoft/skewed-datamatrix
bench_gate "$e_ups" "dynamsoft UPS-Synthetic-Labels" dynamsoft-ups data/dynamsoft/UPS-Synthetic-Labels
bench_gate "$e_pdf" "dynamsoft pdf417-small-module-size" dynamsoft-pdf417 data/dynamsoft/pdf417-small-module-size
bench_gate "$e_barsub" "BarBeR 1/8 subset" barber-subset data/barber --subset 8
bench_gate "$e_barfull" "BarBeR" barber data/barber
bench_gate "$e_zx" "zxing-cpp samples" zxing-samples data/zxing-samples/test/samples
if [ -n "$video" ]; then
    echo "=== dynamsoft videos ($video)"
    gate "$bin/video-bench" data/dynamsoft/video-based-testing --engines "$video" \
        --check "$base/dynamsoft-video.json"
fi
exit "$status"
