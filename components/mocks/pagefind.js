/**
 * Mock Pagefind for development mode.
 * Returns empty results - run a real build to test search.
 */
export async function init() {
  console.log('[pagefind-dev] Mock initialized - run `mbr -b` for real search');
}

export async function options(_opts) {}

export async function search(query) {
  console.log('[pagefind-dev] Search:', query);
  return { results: [] };
}

export async function debouncedSearch(query, _options) {
  return search(query);
}
