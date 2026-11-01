#!/usr/bin/env bash
# Cross-compile whisper.cpp (MIT) for the phone: produces
# build/whisper-arm64/whisper-cli (statically linked ggml, NEON + dotprod).
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
ver="${WHISPER_VERSION:-v1.7.6}"
src="$root/.devtools/whisper.cpp"
[ -d "$src" ] || git clone -q --depth 1 --branch "$ver" https://github.com/ggml-org/whisper.cpp "$src"
image=glissando-cross:forky
podman image exists "$image" || podman build -t "$image" -f "$root/packaging/cross/Containerfile" "$root/packaging/cross"
podman run --rm -v "$root:/src:Z" "$image" sh -c '
set -e
cmake -S /src/.devtools/whisper.cpp -B /src/build/whisper-arm64 \
  -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=aarch64 \
  -DCMAKE_C_COMPILER=aarch64-linux-gnu-gcc -DCMAKE_CXX_COMPILER=aarch64-linux-gnu-g++ \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF \
  -DGGML_CPU_ARM_ARCH=armv8.2-a+dotprod+fp16 -DCMAKE_EXE_LINKER_FLAGS="-Wl,-z,relro,-z,now" -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF >/dev/null
cmake --build /src/build/whisper-arm64 -j"$(nproc)" --target whisper-cli >/dev/null
aarch64-linux-gnu-strip /src/build/whisper-arm64/bin/whisper-cli'
file "$root/build/whisper-arm64/bin/whisper-cli"
