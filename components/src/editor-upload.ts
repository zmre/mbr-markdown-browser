/**
 * Pure helper for the editor's image-upload wiring.
 *
 * Deliberately free of any runtime import so this logic stays cheap and
 * unit-testable. The `fetch`-driven uploader that consumes it lives in
 * editor-crepe.ts (it needs that module's `authHeaders`/`opts` closure).
 */

/**
 * The note's own folder, repo-relative, derived from its source file path — the
 * directory an uploaded asset should land in. Strips the final `/`-segment (the
 * filename) and any leading `/`, so `notes/foo.md` → `notes` and a root-level
 * `foo.md` → `""`.
 */
export function noteDir(filePath: string): string {
  const slash = filePath.lastIndexOf('/');
  if (slash < 0) return '';
  return filePath.slice(0, slash).replace(/^\/+/, '');
}
