export const FIXED_MANIFEST_HEADER_SIZE: number = 256;
export const FIXED_GLOBAL_HEADER_SIZE: number = 96;
export const FIXED_WRAPPER_HEADER_SIZE: number = 64;
export const NIL: number = 4_294_967_295;
export const NO_EXPIRATION_LAST_VALID_SLOT = 0;
export const U32_MAX = 4_294_967_295;
export const PRICE_MIN_EXP = -18;
export const PRICE_MAX_EXP = 8;

/**
 * Duration of a Solana slot in milliseconds.
 *
 * Single source of truth for every slot <-> wall clock conversion in this
 * repo. When the cluster's slot duration changes, update this one constant and
 * nothing else. Do not hardcode a slot duration anywhere else.
 */
export const SLOT_DURATION_MS: number = 350;
/** Slots the cluster produces in a day, at SLOT_DURATION_MS. */
export const SLOTS_PER_DAY: number = Math.round(
  (24 * 60 * 60 * 1_000) / SLOT_DURATION_MS,
);

/** Number of slots the cluster produces in the given duration. */
export function slotsForDurationMs(durationMs: number): number {
  return Math.ceil(durationMs / SLOT_DURATION_MS);
}

/** Wall clock duration, in milliseconds, of the given number of slots. */
export function durationMsForSlots(slots: number): number {
  return slots * SLOT_DURATION_MS;
}
