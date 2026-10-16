/**
 * Character references (`&amp;`, `&copy;`, `&nbsp;`, `&#169;`, `&#xA9;`) and
 * the `\$` escape, kept as authored.
 *
 * remark decodes a character reference into the character it names, and the
 * editor then saves the character: `&nbsp;` becomes an invisible U+00A0,
 * `&lt;b&gt;` becomes `<b>` — which the renderer reads as HTML. And it drops
 * the backslash of `\$`, which since the renderer reads `$…$` as math (see
 * `math.ts`) can turn `\$x$` into a formula. Both are rewrites the author did
 * not make.
 *
 * Instead, in phrasing (paragraphs, headings, cells, emphasis, link text),
 * each becomes an inline `html` node holding its source — Milkdown's existing
 * atom for raw inline HTML, which shows that source in the editor and prints
 * it back verbatim. No new editor node is needed.
 *
 * Elsewhere — link destinations and titles, image alt text, code-fence info
 * strings, reference labels — the decoded value is what those fields hold, so
 * the default (decode) applies there, as it always has.
 *
 * Only the mdast-util-from-markdown handlers change: micromark already
 * tokenizes both constructs; we decide what node they become.
 */
import type { CompileContext, Token } from 'mdast-util-from-markdown';
import type { Html, Nodes, Parents } from 'mdast';
import { $syntax } from './syntax-extension.js';

/** A from-markdown stack entry: a node, or the fragment `buffer()` opens. */
type StackEntry = CompileContext['stack'][number];

/** Fragments that hold a link's text (phrasing), as opposed to a destination or alt. */
const linkTextFragments = new WeakSet<StackEntry>();

function top(context: CompileContext): StackEntry {
  return context.stack[context.stack.length - 1];
}

/** Whether characters here end up as phrasing text in the document. */
function inPhrasing(context: CompileContext): boolean {
  const current = top(context);
  return current.type !== 'fragment' || linkTextFragments.has(current);
}

/** `true` for the raw node this module opened (nothing else leaves an `html` node open). */
function isOurs(entry: StackEntry): entry is Html {
  return entry.type === 'html';
}

/** mdast-util-from-markdown's `onenterdata`: continue the open text node or start one. */
function enterText(context: CompileContext, token: Token): void {
  const parent = top(context) as Parents;
  const siblings = parent.children as Nodes[];
  let tail = siblings[siblings.length - 1];
  if (!tail || tail.type !== 'text') {
    tail = {
      type: 'text',
      value: '',
      position: { start: { ...token.start }, end: { ...token.start } },
    };
    siblings.push(tail);
  }
  context.stack.push(tail);
}

/** Opens an `html` node for `token`'s source; closed by the construct's own exit. */
function enterRaw(context: CompileContext, token: Token): void {
  const node: Html = { type: 'html', value: '' };
  context.enter(node, token);
}

export const escapesSyntax = {
  fromMarkdown: {
    enter: {
      // `buffer` (the default), remembering which fragments are link text.
      label(this: CompileContext) {
        this.buffer();
        if (this.stack[this.stack.length - 2]?.type === 'link') linkTextFragments.add(top(this));
      },
      characterReference(this: CompileContext, token: Token) {
        if (inPhrasing(this)) enterRaw(this, token);
        else enterText(this, token);
      },
      characterEscape(this: CompileContext, token: Token) {
        if (inPhrasing(this) && this.sliceSerialize(token) === '\\$') enterRaw(this, token);
        else enterText(this, token);
      },
    },
    exit: {
      // The default value handler has appended the decoded character to
      // whatever is open; for ours, the source replaces it.
      characterReference(this: CompileContext, token: Token) {
        const current = top(this);
        if (isOurs(current)) {
          current.value = this.sliceSerialize(token);
          this.exit(token);
          return;
        }
        this.stack.pop();
        if (current.position) current.position.end = { ...token.end };
      },
      // The default (`onexitdata`) for everyone else.
      characterEscapeValue(this: CompileContext, token: Token) {
        const current = top(this);
        if (isOurs(current)) return;
        this.stack.pop();
        if (current.type === 'text') current.value += this.sliceSerialize(token);
        if (current.position) current.position.end = { ...token.end };
      },
      characterEscape(this: CompileContext, token: Token) {
        const current = top(this);
        if (!isOurs(current)) return;
        current.value = this.sliceSerialize(token);
        this.exit(token);
      },
    },
  },
};

/** Remark plugin: character references and `\$` keep their source. */
export const escapesPlugin = $syntax('remarkMbrEscapes', escapesSyntax);
