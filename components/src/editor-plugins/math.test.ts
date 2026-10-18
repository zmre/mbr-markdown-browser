/**
 * The editor's math plugin against the renderer's math semantics.
 *
 * `tests/fixtures/editor-math-cases.json` is the shared table: mbr-core's `editor_math_parity` test
 * holds pulldown-cmark (the renderer) to it, and this file holds the editor's
 * parse to it — so the two agree on what is math, case by case. Each case is
 * also saved untouched and must come back byte for byte, which is what the
 * plugin is for: TeX keeps its backslashes.
 */
import { editorViewCtx, remarkCtx } from '@milkdown/kit/core';
import type { EditorView } from '@milkdown/kit/prose/view';
import { afterEach, describe, expect, it } from 'vitest';
import { destroyEditors, openCrepe, savedFile } from '../editor-test-support.js';

interface MathSpan {
  display: boolean;
  tex: string;
}

interface MathCase {
  source: string;
  math: MathSpan[];
}

declare const __MBR_EDITOR_MATH_CASES__: string;

const CASES = JSON.parse(__MBR_EDITOR_MATH_CASES__) as MathCase[];

/**
 * Saves that differ for reasons that are not math (item 6 cosmetics: `-` →
 * `*`, tables re-padded, a lone backtick escaped).
 */
const COSMETIC_ONLY = new Set([
  '- item $$\n  x\n  $$\n',
  '| a | b |\n|---|---|\n| $x$ | $y + z$ |\n',
  'Math first $a`b$ c`.\n',
]);

interface Node {
  type: string;
  value?: string;
  display?: boolean;
  children?: Node[];
}

function collectMath(node: Node, out: MathSpan[] = []): MathSpan[] {
  if (node.type === 'mbrMath') {
    const width = node.display ? 2 : 1;
    const raw = node.value ?? '';
    out.push({ display: node.display === true, tex: raw.slice(width, raw.length - width) });
  }
  for (const child of node.children ?? []) collectMath(child, out);
  return out;
}

async function editorMath(source: string): Promise<MathSpan[]> {
  const crepe = await openCrepe(source);
  const tree = crepe.editor.action((ctx) => {
    const remark = ctx.get(remarkCtx);
    return remark.runSync(remark.parse(source), source);
  }) as unknown as Node;
  return collectMath(tree);
}

afterEach(destroyEditors);

describe('editor math matches the renderer (editor-math-cases.json)', () => {
  it('has the shared table', () => {
    expect(CASES.length).toBeGreaterThan(20);
  });

  for (const { source, math } of CASES) {
    it(`parses ${JSON.stringify(source)}`, async () => {
      expect(await editorMath(source)).toEqual(math);
    });
    if (!COSMETIC_ONLY.has(source)) {
      it(`saves ${JSON.stringify(source)} byte for byte`, async () => {
        expect(await savedFile(source)).toBe(source);
      });
    }
  }
});

describe('editing math', () => {
  /** Runs `edit` with the view and the position just inside the first math node. */
  const editMath = (source: string, edit: (view: EditorView, inside: number, size: number) => void) =>
    savedFile(source, (crepe) => {
      crepe.editor.action((ctx) => {
        const view = ctx.get(editorViewCtx);
        let found: { pos: number; size: number } | undefined;
        view.state.doc.descendants((node, pos) => {
          if (!found && node.type.name === 'mbr_math') found = { pos: pos + 1, size: node.content.size };
          return !found;
        });
        if (!found) throw new Error('no mbr_math node');
        edit(view, found.pos, found.size);
      });
    });

  it('is a node holding the TeX, which edits print verbatim', async () => {
    const saved = await editMath('Area $\\pi r^2$ here.\n', (view, inside, size) => {
      view.dispatch(view.state.tr.insertText('\\,', inside + size));
    });
    expect(saved).toBe('Area $\\pi r^2\\,$ here.\n');
  });

  it('keeps display delimiters when edited', async () => {
    const saved = await editMath('$$\na \\; b\n$$\n', (view, inside) => {
      view.dispatch(view.state.tr.insertText('\\{', inside + 1));
    });
    expect(saved).toBe('$$\n\\{a \\; b\n$$\n');
  });

  it('drops a math node whose TeX was deleted', async () => {
    const saved = await editMath('Gone $x$ here.\n', (view, inside, size) => {
      view.dispatch(view.state.tr.delete(inside, inside + size));
    });
    expect(saved).toBe('Gone  here.\n');
  });
});
