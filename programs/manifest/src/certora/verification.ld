/* Verification-only mutable globals. The v3 default script discards .bss
 * and .data; merging these globals into .rodata would corrupt its permissions.
 * Extend (do not replace) the compiler's v3 script with a separate RW segment.
 */
PHDRS { certora PT_LOAD FLAGS(6); }
SECTIONS {
  .data : ALIGN(8) { *(.certora) } :certora
} INSERT AFTER .rodata;
