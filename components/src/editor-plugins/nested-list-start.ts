/**
 * A list item that starts inside a container opened on the same line, read as
 * pulldown-cmark (the renderer) reads it.
 *
 * ```markdown
 * * a
 *   * 6. x        ← pulldown-cmark: an ordered list starting at 6 in `* `
 *
 * text
 * > 6. x          ← the same, in the quote
 * ```
 *
 * A list item may interrupt a paragraph only when it is a bullet or starts at
 * 1, and is not empty. pulldown-cmark applies that test once per line, to the
 * first new block: the inner `* ` or `> ` above interrupts `a`, and whatever
 * follows on the line opens inside that new container, where there is no
 * paragraph left to interrupt (`firstpass.rs`: the new-container loop never
 * asks). micromark's document tokenizer sets its `interrupt` flag for the first
 * new container and leaves it set for the rest of the line, so its `list`
 * construct refuses `6.` (and an empty item) there, and the editor read
 * `6. x` as paragraph text. That alone keeps an untouched save exact, but an
 * edit printed the text escaped, `6\. x`, and the renderer's list was gone.
 *
 * {@link nestedListStartSyntax} is micromark's own `list` construct (so its
 * continuation and exit are unchanged), tried first, and only on such a line:
 * when the document tokenizer is interrupting and a container has already been
 * opened on the current line, it clears the flag before the list starts.
 */
import { list } from 'micromark-core-commonmark';
import type { Construct, Effects, Extension, State, TokenizeContext } from 'micromark-util-types';
import { $syntax } from './syntax-extension.js';

/**
 * Tokens a *new* container enters on its first line, and a continuation never
 * does: a list item's marker, a quote (its prefix is entered on every line,
 * the `blockQuote` itself only when it opens), a definition, a footnote.
 */
const OPENING_TOKENS: ReadonlySet<string> = new Set([
  'listItemPrefix',
  'blockQuote',
  'defListDescriptionPrefix',
  'gfmFootnoteDefinition',
]);

/** Whether a container has been opened on the line the document tokenizer is on. */
export function openedContainerOnLine(self: TokenizeContext): boolean {
  const line = self.now().line;
  for (let i = self.events.length - 1; i >= 0; i--) {
    const [kind, token] = self.events[i];
    if (token.start.line !== line) return false;
    if (kind === 'enter' && OPENING_TOKENS.has(token.type)) return true;
  }
  return false;
}

function tokenizeNestedListStart(this: TokenizeContext, effects: Effects, ok: State, nok: State): State {
  // Not this case: leave the line to micromark's own `list`, which runs next.
  if (!this.interrupt || !openedContainerOnLine(this)) return nok;
  // The rest of this line opens inside the new container: nothing to interrupt.
  this.interrupt = undefined;
  return list.tokenize.call(this, effects, ok, nok);
}

/** `list`, with the tokenizer above; `before` the core construct for the same codes. */
const nestedListStart: Construct = { ...list, name: 'mbrNestedListStart', tokenize: tokenizeNestedListStart, add: 'before' };

const ASTERISK = 42;
const PLUS = 43;
const DASH = 45;
const DIGITS = Array.from({ length: 10 }, (_, i) => 48 + i);

/** The micromark extension (see the module docs). */
export function nestedListStartSyntax(): Extension {
  const document: Record<number, Construct> = {};
  for (const code of [ASTERISK, PLUS, DASH, ...DIGITS]) document[code] = nestedListStart;
  return { document };
}

/** The Milkdown plugin, for `MARKDOWN_EXTENSION_PLUGINS`. */
export const nestedListStartPlugin = $syntax('mbrNestedListStart', { micromark: nestedListStartSyntax() });
