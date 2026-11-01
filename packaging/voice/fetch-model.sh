#!/usr/bin/env bash
# Download the speech model packaged in glissando-voice into .devtools/voice,
# pinned to one upstream revision and checked by SHA-256.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
rev=5359861c739e955e79d9a303bcbc70fb988958b1
sha256=c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9
model="$root/.devtools/voice/ggml-base-q8_0.bin"
mkdir -p "$(dirname "$model")"
[ -f "$model" ] || curl -fsSL --retry 3 -o "$model" \
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/$rev/ggml-base-q8_0.bin"
echo "$sha256  $model" | sha256sum --check --quiet
