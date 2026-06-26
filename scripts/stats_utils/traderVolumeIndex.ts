/**
 * Keeps traders ordered by total notional volume (descending) so the stats
 * server can read the top-N for the `/traders` endpoint without re-sorting
 * every tracked trader on each request.
 *
 * It exploits the fact that a trader's notional volume only ever increases:
 * an update is an "increase-key", so the element only ever moves toward the
 * front. Updates that don't change the ordering (the common case) are O(1) -
 * just an in-place total bump and a single neighbor comparison. A reorder
 * costs only the number of positions actually jumped, which is small in steady
 * state because each fill's notional is tiny relative to the leaders' totals.
 * Reading the top N is an O(N) slice of an already-sorted array.
 *
 * Ordering must mirror the previous full-sort behavior, which sorted by
 * `takerNotionalVolume + makerNotionalVolume` descending, so this index tracks
 * that sum. It includes every trader the server tracks (even zero-volume ones,
 * via `ensure`) so the top-N result is identical to the old map/sort/slice.
 */
export class TraderVolumeIndex {
  // Traders sorted by total volume, descending. order[0] is the largest.
  private order: string[] = [];
  // trader -> its current index into `order`.
  private pos: Map<string, number> = new Map();
  // trader -> current total notional volume (taker + maker).
  private totals: Map<string, number> = new Map();

  /** Ensure a trader is tracked, inserting it at volume 0 if new. */
  ensure(trader: string): void {
    if (this.pos.has(trader)) {
      return;
    }
    this.totals.set(trader, 0);
    this.pos.set(trader, this.order.length);
    this.order.push(trader); // a zero total belongs at the back
  }

  /**
   * Add `delta` to a trader's total volume and restore sorted order. `delta`
   * is expected to be non-negative (volume only accumulates); a zero/negative
   * delta updates the total but never reorders.
   */
  add(trader: string, delta: number): void {
    if (!this.pos.has(trader)) {
      this.ensure(trader);
    }
    const newTotal: number = this.totals.get(trader)! + delta;
    this.totals.set(trader, newTotal);
    if (delta <= 0) {
      return;
    }
    // Bubble toward the front while the predecessor is now smaller. Strict `<`
    // leaves equal-volume neighbors in place (stable for ties).
    let i: number = this.pos.get(trader)!;
    while (i > 0 && this.totals.get(this.order[i - 1])! < newTotal) {
      const prev: string = this.order[i - 1];
      this.order[i - 1] = trader;
      this.order[i] = prev;
      this.pos.set(prev, i);
      i--;
    }
    this.pos.set(trader, i);
  }

  /** Current total volume tracked for a trader (0 if untracked). */
  getTotal(trader: string): number {
    return this.totals.get(trader) || 0;
  }

  /** Number of traders tracked. */
  get size(): number {
    return this.order.length;
  }

  /** The top `n` traders by volume, in descending order. */
  top(n: number): string[] {
    return this.order.slice(0, Math.max(0, n));
  }

  /**
   * Replace all contents with the given trader/total pairs. Used after a bulk
   * mutation (e.g. pruning inactive traders) where incremental updates would be
   * more error-prone than a one-shot rebuild.
   */
  rebuild(entries: Array<[string, number]>): void {
    const sorted: Array<[string, number]> = entries
      .slice()
      .sort((a: [string, number], b: [string, number]): number => b[1] - a[1]);
    this.order = sorted.map(([trader]: [string, number]): string => trader);
    this.pos = new Map<string, number>(
      this.order.map((trader: string, i: number): [string, number] => [
        trader,
        i,
      ]),
    );
    this.totals = new Map<string, number>(sorted);
  }
}
