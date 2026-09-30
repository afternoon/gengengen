/* Workshop System Computer: RP2040 on the module, SPI flash on the program card.
   Cards are 2MB or 16MB; this targets 2MB. For a 16MB card, raise FLASH length. */
MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10000100, LENGTH = 2048K - 0x100
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K
}
