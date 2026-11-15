/**
 * The editor mode the reader last chose — the rich editor or the markdown
 * source — so a reader who prefers one gets it the next time the editor opens
 * (editor chunk).
 *
 * `localStorage[EDITOR_MODE_KEY] = "source"` while source mode is preferred;
 * removed for the rich editor, the default. Not scoped to a repository: it is
 * how this reader likes to edit on this device, the same in every vault.
 *
 * A UI preference, not a credential: one word, nothing from the page or the
 * note. It is on `edit-token.test.ts`'s `localStorage` allowlist for that
 * reason, and calls `localStorage` itself (a wrapper would hide it from that
 * scan). The edit token never comes near it.
 */

export type EditorMode = 'rich' | 'source';

/** The storage key. */
export const EDITOR_MODE_KEY = 'mbr_editor_mode';

/** The mode to open in: `source` when the reader last chose it, else `rich`. */
export function preferredEditorMode(): EditorMode {
  try {
    return localStorage.getItem(EDITOR_MODE_KEY) === 'source' ? 'source' : 'rich';
  } catch {
    // Storage refused (private mode, file://): the default.
    return 'rich';
  }
}

/** Remembers `mode` as the reader's choice. */
export function rememberEditorMode(mode: EditorMode): void {
  try {
    if (mode === 'source') localStorage.setItem(EDITOR_MODE_KEY, 'source');
    else localStorage.removeItem(EDITOR_MODE_KEY);
  } catch {
    // Storage refused: the choice lasts as long as this editor.
  }
}
