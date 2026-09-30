#!/usr/bin/env bash
# Build the firmware and produce a UF2 for drag-and-drop flashing.
set -euo pipefail

cd "$(dirname "$0")"

echo "==> tests (host)"
cargo test --target "$(rustc -vV | sed -n 's/^host: //p')" --lib

echo "==> firmware (thumbv6m-none-eabi)"
cargo build --release

ELF=target/thumbv6m-none-eabi/release/gengengen
echo "==> uf2"
python3 tools/elf2uf2.py "$ELF" gengengen.uf2

echo
echo "Flash it:"
echo "  1. Hold the BOOT button (behind the top knob - pull the knob off)"
echo "  2. Connect USB; the Computer appears as a USB drive"
echo "  3. Copy gengengen.uf2 onto it, then eject"
echo "  4. Tap the reset button next to the card slot"
