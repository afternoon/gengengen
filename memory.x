/* Workshop System Computer: RP2040 on the module, SPI flash on the program card.
   Cards are 2MB or 16MB; this targets 2MB. For a 16MB card, raise FLASH length.

   The second-stage bootloader must sit at the very start of flash, where the
   RP2040 bootrom looks for it, so BOOT2 is its own region and the .boot2 section
   is placed into it explicitly below. Without that placement the section lands
   after .data and the image is unbootable (and elf2uf2-rs rejects it). */
MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10000100, LENGTH = 2048K - 0x100
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K
}

SECTIONS {
    .boot2 ORIGIN(BOOT2) :
    {
        KEEP(*(.boot2));
    } > BOOT2
} INSERT BEFORE .text;
