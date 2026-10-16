/**
 * Math (`$…$` inline, `$$…$$` display) for the editor, with **pulldown-cmark's
 * `ENABLE_MATH` rules** — the renderer's — rather than remark-math's, and
 * printed back byte for byte.
 *
 * Without it the editor reads math as text, so CommonMark backslash escapes
 * inside it are consumed on save (`\,` → `,`, `\{a\}` → `{a}`): silent LaTeX
 * damage. Crepe's own `Latex` feature is not the answer: it needs CodeMirror,
 * parses with remark-math (whose `$5 and $10` is math and whose `$$` is a
 * block), and saves display math as a ```` ```LaTeX ```` fence.
 *
 * # What is math (pulldown-cmark 0.13, `firstpass.rs` / `parse.rs`)
 *
 * Math is inline, inside a paragraph (or heading, cell, …) — a `$$` block on
 * its own lines is a paragraph holding display math, and so it is here.
 * - `$` opens inline math when the next character exists and is not
 *   whitespace; `$$` opens display math (no whitespace rule).
 * - The span closes at the first `$` in the same brace context: braces
 *   `{…}` nest, a `$` inside them is content (`$\text{$x$}$`), and once an
 *   unbalanced `}` closes the opener's context nothing can close it. Inline
 *   math needs that `$` to follow a non-whitespace character; display math
 *   needs it to be `$$`. Any other `$` at the opener's depth means no math —
 *   which is why `Price is $5 and $10` is text.
 * - `\` + ASCII punctuation is an escape for the scan (`\$`, `\{` neither
 *   close nor nest) but stays in the math source verbatim.
 * - A failed `$$` leaves its second `$` free to open inline math, as
 *   micromark retries the construct at the next `$` by itself.
 *
 * The source between the delimiters is kept as authored (line endings
 * included; container prefixes such as `> ` are the container's and are
 * re-added on print).
 *
 * # In the editor
 *
 * `mbr_math` is an inline node whose content is the LaTeX source as plain
 * text (`code`: no marks, no input rules), shown in monospace between its
 * `$`/`$$` delimiters, so it is edited in place. It prints as
 * delimiter + text + delimiter; an emptied inline span prints nothing.
 */
import { $nodeSchema } from '@milkdown/kit/utils';
import type { Literal } from 'mdast';
import type { CompileContext, Token } from 'mdast-util-from-markdown';
import type { Code, Construct, Effects, State, TokenizeContext } from 'micromark-util-types';
import { $syntax } from './syntax-extension.js';

declare module 'micromark-util-types' {
  interface TokenTypeMap {
    mbrMath: 'mbrMath';
    mbrMathSequence: 'mbrMathSequence';
    mbrMathData: 'mbrMathData';
  }
}

/** mdast node for a math span: `value` is the whole source, delimiters included. */
export interface MbrMath extends Literal {
  type: 'mbrMath';
  display: boolean;
}

declare module 'mdast' {
  interface PhrasingContentMap {
    mbrMath: MbrMath;
  }
  interface RootContentMap {
    mbrMath: MbrMath;
  }
}

const DOLLAR = 36;
const BACKSLASH = 92;
const LEFT_BRACE = 123;
const RIGHT_BRACE = 125;

/** micromark's line-ending codes: CR, LF, CRLF. */
function isLineEnding(code: Code): boolean {
  return code === -5 || code === -4 || code === -3;
}

/** pulldown-cmark's `is_ascii_whitespace` (space, tab, LF, VT, FF, CR), in micromark codes. */
function isWhitespace(code: Code): boolean {
  return code === 32 || code === -2 || code === -1 || code === 11 || code === 12 || isLineEnding(code);
}

function isAsciiPunctuation(code: Code): boolean {
  return (
    code !== null &&
    ((code >= 33 && code <= 47) || (code >= 58 && code <= 64) || (code >= 91 && code <= 96) || (code >= 123 && code <= 126))
  );
}

function tokenizeMath(this: TokenizeContext, effects: Effects, ok: State, nok: State): State {
  let display = false;
  // Brace depth relative to the opener: a `$` closes only at 0.
  let depth = 0;
  // Whether the last character of the source so far is whitespace.
  let afterWhitespace = false;
  return start;

  function start(code: Code): State | undefined {
    effects.enter('mbrMath');
    effects.enter('mbrMathSequence');
    effects.consume(code);
    return open;
  }

  function open(code: Code): State | undefined {
    if (code === DOLLAR) {
      display = true;
      effects.consume(code);
      effects.exit('mbrMathSequence');
      return between;
    }
    effects.exit('mbrMathSequence');
    if (code === null || isWhitespace(code)) return nok(code);
    return between(code);
  }

  function between(code: Code): State | undefined {
    if (code === null) return nok(code);
    if (code === DOLLAR && depth === 0) return close(code);
    if (isLineEnding(code)) {
      effects.enter('lineEnding');
      effects.consume(code);
      effects.exit('lineEnding');
      afterWhitespace = true;
      return between;
    }
    effects.enter('mbrMathData');
    return data(code);
  }

  function data(code: Code): State | undefined {
    if (code === null || isLineEnding(code) || (code === DOLLAR && depth === 0)) {
      effects.exit('mbrMathData');
      return between(code);
    }
    if (code === BACKSLASH) {
      effects.consume(code);
      afterWhitespace = false;
      return escape;
    }
    if (code === LEFT_BRACE) depth += 1;
    else if (code === RIGHT_BRACE) {
      depth -= 1;
      // The opener's brace context is gone for good: nothing can close it.
      if (depth < 0) return nok(code);
    }
    afterWhitespace = isWhitespace(code);
    effects.consume(code);
    return data;
  }

  function escape(code: Code): State | undefined {
    if (isAsciiPunctuation(code)) {
      effects.consume(code);
      return data;
    }
    return data(code);
  }

  function close(code: Code): State | undefined {
    // Inline: the first `$` at depth 0 must be able to close, or this is not math.
    if (!display && afterWhitespace) return nok(code);
    effects.enter('mbrMathSequence');
    effects.consume(code);
    return display ? closeDisplay : done;
  }

  function closeDisplay(code: Code): State | undefined {
    if (code !== DOLLAR) return nok(code);
    effects.consume(code);
    return done;
  }

  function done(code: Code): State | undefined {
    effects.exit('mbrMathSequence');
    effects.exit('mbrMath');
    return ok(code);
  }
}

const mathConstruct: Construct = { name: 'mbrMath', tokenize: tokenizeMath };

/** The micromark / mdast halves of the syntax (exported for unit tests). */
export const mathSyntax = {
  micromark: { text: { [DOLLAR]: mathConstruct } },
  fromMarkdown: {
    enter: {
      mbrMath(this: CompileContext, token: Token) {
        this.enter({ type: 'mbrMath', value: '', display: false }, token);
      },
    },
    exit: {
      mbrMath(this: CompileContext, token: Token) {
        const node = this.stack[this.stack.length - 1] as MbrMath;
        node.value = this.sliceSerialize(token);
        node.display = node.value.startsWith('$$');
        this.exit(token);
      },
    },
  },
  toMarkdown: {
    handlers: {
      mbrMath: Object.assign((node: MbrMath) => node.value, { peek: () => '$' }),
    },
  },
};

/** Remark plugin: the math syntax. */
export const remarkMbrMath = $syntax('remarkMbrMath', mathSyntax);

/** The editor node: the LaTeX source as text, between its delimiters. */
export const mathSchema = $nodeSchema('mbr_math', () => ({
  group: 'inline',
  inline: true,
  content: 'text*',
  marks: '',
  code: true,
  attrs: { display: { default: false } },
  parseDOM: [
    {
      tag: 'span[data-type="mbr-math"]',
      contentElement: 'span[data-mbr-math-source]',
      getAttrs: (dom: HTMLElement) => ({ display: dom.dataset.display === 'true' }),
    },
  ],
  toDOM: (node) => {
    const delimiter = node.attrs.display ? '$$' : '$';
    const fence = ['span', { contenteditable: 'false', style: 'opacity: 0.55' }, delimiter] as const;
    return [
      'span',
      {
        'data-type': 'mbr-math',
        'data-display': node.attrs.display ? 'true' : 'false',
        class: 'mbr-editor-math',
        style: 'font-family: var(--crepe-font-code, monospace); font-size: 0.9em; white-space: pre-wrap;',
      },
      fence,
      ['span', { 'data-mbr-math-source': '' }, 0],
      fence,
    ];
  },
  parseMarkdown: {
    match: (node) => node.type === 'mbrMath',
    runner: (state, node, type) => {
      const raw = String(node.value ?? '');
      const display = node.display === true;
      const width = display ? 2 : 1;
      const source = raw.slice(width, raw.length - width);
      // `addNode` with ready-made content, not `openNode`/`addText`/`closeNode`:
      // closing a node resets the parser's open marks (it is meant for
      // blocks), which would drop a surrounding link from the rest of its text,
      // and `addText` would put those marks on the TeX.
      state.addNode(type, { display }, source ? [type.schema.text(source)] : []);
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === 'mbr_math',
    runner: (state, node) => {
      const source = node.textContent;
      // An emptied inline span is gone (`$$` would open display math); an
      // empty display span is valid and prints as authored.
      if (!source && !node.attrs.display) return;
      const delimiter = node.attrs.display ? '$$' : '$';
      state.addNode('mbrMath', undefined, delimiter + source + delimiter);
    },
  },
}));

/** Everything the editor needs for math. */
export const mathPlugin = [remarkMbrMath, mathSchema].flat();
