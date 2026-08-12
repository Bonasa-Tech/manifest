export function safeAtomsToNumber(atoms: string | bigint): number {
  const exactAtoms: bigint = typeof atoms === 'bigint' ? atoms : BigInt(atoms);
  if (
    exactAtoms > BigInt(Number.MAX_SAFE_INTEGER) ||
    exactAtoms < BigInt(Number.MIN_SAFE_INTEGER)
  ) {
    throw new RangeError(`Atom value is outside the safe integer range: ${atoms}`);
  }
  return Number(exactAtoms);
}
