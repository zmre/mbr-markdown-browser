/**
 * Shared, stateless fuzzy-matching helpers.
 *
 * The scoring algorithm was extracted verbatim from `mbr-fuzzy-nav.ts`'s
 * `_fuzzyScore` so it can be reused by the editor pickers (path / media / link
 * autocomplete) without importing the stateful nav component or `shared.ts`.
 * Keeping it pure and dependency-free lets both the main bundle and the lazy
 * editor chunk share it, and makes it unit-testable in isolation.
 */

/** Escapes a string for safe use inside a `RegExp`. */
export function escapeRegex(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Fuzzy search scoring algorithm. Higher scores = better matches; a score of
 * `0` means "no match" (the query's characters do not appear in order).
 *
 * Scoring:
 * - Exact prefix: 1500
 * - Word-start substring: 1200
 * - Any substring: 1000
 * - Character-by-character subsequence: sum of position/consecutive/word-
 *   boundary bonuses (always > 0 when every query char is found in order)
 */
export function fuzzyScore(text: string, query: string): number {
  if (!query) return 0;

  const lowerText = text.toLowerCase();
  const lowerQuery = query.toLowerCase();

  // Exact substring match (highest priority)
  if (lowerText.includes(lowerQuery)) {
    // Bonus for exact match at start
    if (lowerText.startsWith(lowerQuery)) {
      return 1500;
    }
    // Bonus for word-start match
    const wordStart = new RegExp(`\\b${escapeRegex(lowerQuery)}`);
    if (wordStart.test(lowerText)) {
      return 1200;
    }
    return 1000;
  }

  // Character-by-character fuzzy matching
  let score = 0;
  let textIndex = 0;
  let consecutiveBonus = 0;

  for (const char of lowerQuery) {
    const foundIndex = lowerText.indexOf(char, textIndex);
    if (foundIndex === -1) {
      return 0; // Character not found, no match
    }

    // Bonus for consecutive characters
    if (foundIndex === textIndex) {
      consecutiveBonus += 10;
    } else {
      consecutiveBonus = 0;
    }

    // Base score + position bonus (earlier = better) + consecutive bonus
    score += 10 + Math.max(0, 50 - foundIndex) + consecutiveBonus;

    // Bonus for word boundary match
    if (foundIndex === 0 || /\W/.test(lowerText[foundIndex - 1])) {
      score += 25;
    }

    textIndex = foundIndex + 1;
  }

  return score;
}

/** A candidate paired with the field(s) used to fuzzy-match against. */
export interface FuzzyCandidate<T> {
  item: T;
  /** Text searched against the query (the strongest field score wins). */
  haystacks: string[];
}

/**
 * Fuzzy-filter and rank a list of candidates by `query`, returning the matching
 * items (score > 0) sorted best-first. When `query` is empty the original order
 * is preserved (no filtering). Each candidate is scored against the best of its
 * `haystacks`, so an item can match on either its path or its title.
 */
export function fuzzyFilter<T>(
  candidates: ReadonlyArray<FuzzyCandidate<T>>,
  query: string,
): T[] {
  if (!query.trim()) {
    return candidates.map((c) => c.item);
  }
  return candidates
    .map((c) => ({
      item: c.item,
      score: Math.max(...c.haystacks.map((h) => fuzzyScore(h, query))),
    }))
    .filter(({ score }) => score > 0)
    .sort((a, b) => b.score - a.score)
    .map(({ item }) => item);
}
