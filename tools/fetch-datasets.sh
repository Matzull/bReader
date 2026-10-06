#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$root/data/dynamsoft"
url="https://github.com/Dynamsoft/datasets-from-dynamsoft"

if [ -d "$dest/.git" ]; then
    git -C "$dest" pull --ff-only
else
    mkdir -p "$(dirname "$dest")"
    git clone --depth 1 "$url" "$dest"
fi
