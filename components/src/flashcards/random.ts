/**
 * Injected randomness for the deck chunk.
 *
 * Every random decision the deck makes — Random mode's order, Concentric's
 * picks and placements, the order of new cards in a spaced-repetition session —
 * takes an {@link Rng} rather than calling `Math.random` itself, so the pure
 * modules stay reproducible under a seeded rng in tests.
 */

/** The shape of `Math.random`: a float in `[0, 1)`. */
export type Rng = () => number

/** A shuffled copy of `items` (Fisher–Yates). */
export function shuffle<T>(items: readonly T[], rng: Rng): T[] {
  const out = items.slice()
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(rng() * (i + 1))
    ;[out[i], out[j]] = [out[j], out[i]]
  }
  return out
}
