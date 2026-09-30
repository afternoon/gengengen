#!/usr/bin/env python3
"""Convert an RP2040 ELF into a UF2 image for drag-and-drop flashing.

Why this exists rather than `elf2uf2-rs`: current Rust LLD emits ELF headers with
EI_OSABI = 3 (Linux) for bare-metal ARM targets, and elf2uf2-rs only accepts 0
(SysV/None), so it rejects our perfectly good binary with "Unrecognized ABI".
The UF2 container is simple and specified, so we write it directly.

UF2 spec: https://github.com/microsoft/uf2
"""

import struct
import sys

UF2_MAGIC_START0 = 0x0A324655  # "UF2\n"
UF2_MAGIC_START1 = 0x9E5D5157
UF2_MAGIC_END = 0x0AB16F30
UF2_FLAG_FAMILY_ID = 0x00002000
RP2040_FAMILY_ID = 0xE48BFF56

# UF2 blocks carry 256 payload bytes each, in a fixed 512-byte block.
PAYLOAD_SIZE = 256
BLOCK_SIZE = 512

# The RP2040's memory-mapped flash window. Only loadable segments inside this
# range belong in the image; RAM segments are loaded by the startup code.
FLASH_START = 0x10000000
FLASH_END = 0x15000000


def read_loadable_segments(path):
    """Yield (physical_addr, data) for each PT_LOAD segment with content."""
    with open(path, "rb") as f:
        elf = f.read()

    if elf[:4] != b"\x7fELF":
        raise SystemExit(f"{path}: not an ELF file")
    if elf[4] != 1 or elf[5] != 1:
        raise SystemExit(f"{path}: expected 32-bit little-endian ELF")

    (e_machine,) = struct.unpack_from("<H", elf, 18)
    if e_machine != 40:
        raise SystemExit(f"{path}: expected ARM (e_machine 40), got {e_machine}")

    e_phoff, = struct.unpack_from("<I", elf, 28)
    e_phentsize, e_phnum = struct.unpack_from("<HH", elf, 42)

    segments = []
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        p_type, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz = struct.unpack_from(
            "<IIIIII", elf, off
        )
        if p_type != 1 or p_filesz == 0:  # PT_LOAD with content
            continue
        # Use the physical address: .data lives at a RAM vaddr but is stored in
        # flash at its paddr, and it is the flash location we must write.
        if not (FLASH_START <= p_paddr < FLASH_END):
            continue
        segments.append((p_paddr, elf[p_offset : p_offset + p_filesz]))

    if not segments:
        raise SystemExit(f"{path}: no loadable flash segments found")
    return sorted(segments)


def build_uf2(segments):
    """Flatten segments into 256-byte UF2 blocks."""
    # Merge into one contiguous image, zero-filling any gaps between segments.
    lowest = min(addr for addr, _ in segments)
    highest = max(addr + len(data) for addr, data in segments)
    image = bytearray(highest - lowest)
    for addr, data in segments:
        image[addr - lowest : addr - lowest + len(data)] = data

    # Pad to a whole number of payload blocks.
    if len(image) % PAYLOAD_SIZE:
        image.extend(b"\x00" * (PAYLOAD_SIZE - len(image) % PAYLOAD_SIZE))

    num_blocks = len(image) // PAYLOAD_SIZE
    out = bytearray()
    for i in range(num_blocks):
        chunk = image[i * PAYLOAD_SIZE : (i + 1) * PAYLOAD_SIZE]
        header = struct.pack(
            "<IIIIIIII",
            UF2_MAGIC_START0,
            UF2_MAGIC_START1,
            UF2_FLAG_FAMILY_ID,
            lowest + i * PAYLOAD_SIZE,
            PAYLOAD_SIZE,
            i,
            num_blocks,
            RP2040_FAMILY_ID,
        )
        block = header + chunk
        # Pad the block body out to 512 bytes, then the end magic.
        block += b"\x00" * (BLOCK_SIZE - len(block) - 4)
        block += struct.pack("<I", UF2_MAGIC_END)
        assert len(block) == BLOCK_SIZE
        out += block

    return bytes(out), lowest, len(image)


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: elf2uf2.py <input.elf> <output.uf2>")
    src, dst = sys.argv[1], sys.argv[2]

    segments = read_loadable_segments(src)
    uf2, base, size = build_uf2(segments)

    with open(dst, "wb") as f:
        f.write(uf2)

    print(f"{dst}: {len(uf2)} bytes, {len(uf2)//BLOCK_SIZE} blocks")
    print(f"  flash image: {size} bytes at 0x{base:08x}")
    for addr, data in segments:
        print(f"  segment 0x{addr:08x} + {len(data)}")


if __name__ == "__main__":
    main()
