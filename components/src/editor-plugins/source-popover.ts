/**
 * A one-field popover for editing the markdown source of an atom node — the
 * `{…}` of a section break, the `[[…]]` of a wikilink — anchored under the
 * node's chip.
 *
 * Shared by the editor plugins in this directory; lives in the editor chunk.
 * At most one is open at a time. `Enter` validates and commits, `Escape` and a
 * pointer press outside cancel. `Escape` is stopped here so the editor's own
 * document-level `Escape` (which closes the whole editor) never sees it.
 */

const STYLE_ID = 'mbr-source-popover-style';

const POPOVER_CSS = `
.mbr-source-popover {
  position: fixed; z-index: 2200; display: flex; flex-direction: column; gap: 0.3rem;
  min-width: 16rem; max-width: min(32rem, calc(100vw - 1rem)); padding: 0.5rem;
  background: var(--pico-background-color, #fff); color: var(--pico-color, #1a1a1a);
  border: 1px solid var(--pico-muted-border-color, #ccc); border-radius: 6px;
  box-shadow: 0 8px 28px rgba(0,0,0,0.28); font-size: 0.85rem;
}
.mbr-source-popover label { font-size: 0.75rem; opacity: 0.75; margin: 0; }
.mbr-source-popover input {
  margin: 0; padding: 0.3rem 0.45rem; height: auto;
  font-family: var(--pico-font-family-monospace, monospace); font-size: 0.85rem;
}
.mbr-source-popover .hint { font-size: 0.72rem; opacity: 0.65; }
.mbr-source-popover .hint.error { color: var(--pico-del-color, #c62828); opacity: 1; }
@media (max-width: 600px) { .mbr-source-popover input { font-size: 16px; } }
`;

/** What the popover edits and how it decides a value is acceptable. */
export interface SourcePopoverOptions {
  /** The element the popover hangs under (the node's chip). */
  anchor: HTMLElement;
  /** Field label, e.g. "Section attributes". */
  label: string;
  /** Initial field value. */
  value: string;
  /** Shown under the field while the value is acceptable. */
  hint: string;
  /** `null` when `value` may be committed, else the message to show. */
  validate: (value: string) => string | null;
  /** Called with an accepted value; the popover closes first. */
  onCommit: (value: string) => void;
  /** Called after the popover closes without a commit. */
  onCancel?: () => void;
}

let closeCurrent: (() => void) | null = null;

function ensureStyle(): void {
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  style.textContent = POPOVER_CSS;
  document.head.appendChild(style);
}

/** Opens the popover (closing any other) and returns a function that closes it. */
export function openSourcePopover(options: SourcePopoverOptions): () => void {
  closeCurrent?.();
  ensureStyle();

  const root = document.createElement('div');
  root.className = 'mbr-source-popover';
  root.setAttribute('role', 'dialog');
  root.setAttribute('aria-label', options.label);
  const label = document.createElement('label');
  label.textContent = options.label;
  const input = document.createElement('input');
  input.type = 'text';
  input.value = options.value;
  input.spellcheck = false;
  input.setAttribute('autocapitalize', 'off');
  input.setAttribute('autocomplete', 'off');
  const hint = document.createElement('div');
  hint.className = 'hint';
  hint.textContent = options.hint;
  label.appendChild(input);
  root.append(label, hint);
  document.body.appendChild(root);

  const rect = options.anchor.getBoundingClientRect();
  const width = root.offsetWidth || 256;
  root.style.left = `${Math.max(8, Math.min(rect.left, window.innerWidth - width - 8))}px`;
  root.style.top = `${Math.round(rect.bottom + 4)}px`;

  let closed = false;
  const close = (committed: boolean): void => {
    if (closed) return;
    closed = true;
    document.removeEventListener('mousedown', onOutside, true);
    root.remove();
    if (closeCurrent === cancel) closeCurrent = null;
    if (!committed) options.onCancel?.();
  };
  const cancel = (): void => close(false);
  const onOutside = (event: MouseEvent): void => {
    if (!root.contains(event.target as Node)) cancel();
  };

  input.addEventListener('input', () => {
    const error = options.validate(input.value);
    hint.textContent = error ?? options.hint;
    hint.classList.toggle('error', error !== null);
  });
  input.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      cancel();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      const error = options.validate(input.value);
      if (error !== null) {
        hint.textContent = error;
        hint.classList.add('error');
        return;
      }
      close(true);
      options.onCommit(input.value);
    }
  });
  document.addEventListener('mousedown', onOutside, true);
  closeCurrent = cancel;
  input.focus();
  input.select();
  return cancel;
}
