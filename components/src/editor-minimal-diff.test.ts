/**
 * The minimal-diff save (`editor-minimal-diff.ts`): the pure pieces, then the
 * real editor — including a randomized check over the fixture corpus that an
 * edit rewrites its own block and nothing else.
 */
import { afterEach, describe, expect, it } from 'vitest';
import type { Crepe } from '@milkdown/crepe';
import { remarkCtx } from '@milkdown/kit/core';
import { TextSelection } from '@milkdown/kit/prose/state';
import { changedLines, createBodySaver, serializeBody } from './editor-crepe.js';
import { splitFrontmatter } from './editor-frontmatter.js';
import {
  formatLineRanges,
  matchSequences,
  minimalDiff,
  prepareBaseline,
  splitBlocks,
  type ParsedRoot,
} from './editor-minimal-diff.js';
import { EDITOR_FIXTURES, destroyEditors, fixture, openCrepe, placeCursor, typeText, viewOf } from './editor-test-support.js';

afterEach(destroyEditors);

function parserOf(crepe: Crepe): (markdown: string) => ParsedRoot {
  const remark = crepe.editor.action((ctx) => ctx.get(remarkCtx));
  return (markdown) => remark.parse(markdown) as unknown as ParsedRoot;
}

describe('pure pieces', () => {
  it('matchSequences finds a longest common subsequence, prefix and suffix first', () => {
    expect(matchSequences(['a', 'b', 'c', 'd'], ['a', 'x', 'c', 'd'])).toEqual([
      [0, 0],
      [2, 2],
      [3, 3],
    ]);
    expect(matchSequences(['a', 'b'], ['b', 'a', 'b'])).toEqual([
      [0, 1],
      [1, 2],
    ]);
    expect(matchSequences([], ['a'])).toEqual([]);
  });

  it('formats line ranges', () => {
    expect(formatLineRanges([{ first: 3, last: 3 }, { first: 5, last: 7 }], 2)).toBe('5, 7–9');
  });

  it('changedLines reports the lines a rewrite loses', () => {
    expect(changedLines('a\nb\nc\nd\n', 'a\nB\nC\nd\n')).toEqual([{ first: 2, last: 3 }]);
  });
});

describe('splitting and pairing with the editor parser', () => {
  it('splits at top-level blocks, keeping leading, gaps and trailing', async () => {
    const parse = parserOf(await openCrepe(''));
    const split = splitBlocks('\n# H\n\npara\nmore\n\n\n- a\n- b\n', parse);
    expect(split).toEqual({
      leading: '\n',
      blocks: ['# H', 'para\nmore', '- a\n- b'],
      gaps: ['\n\n', '\n\n\n'],
      trailing: '\n',
      lines: [2, 4, 8],
    });
  });

  it('pairs changed blocks by position between anchors', async () => {
    const parse = parserOf(await openCrepe(''));
    const base = prepareBaseline('# A\n\nT\n:x\n\nB\n', '# A\n\nT\n: x\n\nB\n', parse, (md) => md);
    expect(base?.toOriginal).toEqual([0, 1, 2]);
  });
});

/** Opens `source`'s body, edits it, and returns what a save writes and how. */
async function saveAfter(source: string, edit?: (crepe: Crepe) => void): Promise<{ body: string; whole: boolean; original: string; crepe: Crepe }> {
  const { body: original } = splitFrontmatter(source);
  const crepe = await openCrepe(original);
  const saver = createBodySaver(crepe, original);
  edit?.(crepe);
  return { ...saver.body(), original, crepe };
}

describe('the save', () => {
  it('writes an untouched body back byte for byte, even where the round trip is not exact', async () => {
    const source = fixture('deflist-edge');
    const { body, whole, original } = await saveAfter(source);
    expect(whole).toBe(false);
    expect(body).toBe(original);
  });

  it('rewrites only the edited block', async () => {
    const source = 'Intro *x*\n\nTerm\n:no space\n\n+ plus\n+ list\n';
    const { body } = await saveAfter(source, (crepe) => {
      const view = viewOf(crepe);
      placeCursor(view, 'Intro', 5);
      typeText(view, '!');
    });
    expect(body).toBe('Intro! *x*\n\nTerm\n:no space\n\n+ plus\n+ list\n');
  });

  it('a new block between two originals keeps their own spacing around it', async () => {
    const source = 'One\n\n\n\nTwo\n';
    const { body } = await saveAfter(source, (crepe) => {
      const view = viewOf(crepe);
      placeCursor(view, 'One', 3);
      const at = view.state.selection.$from.after();
      view.dispatch(view.state.tr.insert(at, view.state.schema.nodes.paragraph.create(null, view.state.schema.text('New'))));
    });
    expect(body).toBe('One\n\nNew\n\nTwo\n');
  });

  it('closes an unclosed fence once something follows it, so nothing is swallowed', async () => {
    // An unclosed fence runs to the end of the file. It is printed as typed
    // while it ends the note, and closed once a block follows it: the
    // original (unclosed) spelling would make the new paragraph code.
    const source = 'Para\n\n```\ncode\n';
    const { body, whole } = await saveAfter(source, (c) => {
      const view = viewOf(c);
      const end = view.state.doc.content.size;
      view.dispatch(view.state.tr.insert(end, view.state.schema.nodes.paragraph.create(null, view.state.schema.text('After'))));
    });
    expect(whole).toBe(false);
    expect(body).toBe('Para\n\n```\ncode\n```\n\nAfter\n');
  });

  it('reports the lines the round trip changes', async () => {
    // A lazy continuation line in a list item is printed indented.
    const source = 'Fine\n\n- item\nlazy line\n\nAlso fine\n';
    const crepe = await openCrepe(source);
    expect(createBodySaver(crepe, source).rewritten).toEqual({ ifEdited: [{ first: 3, last: 4 }], onSave: [] });
  });

  it('reports nothing for an exact round trip', async () => {
    const crepe = await openCrepe('Fine\n');
    expect(createBodySaver(crepe, 'Fine\n').rewritten).toEqual({ ifEdited: [], onSave: [] });
  });

  it('minimalDiff returns null rather than a body that splits differently', async () => {
    const crepe = await openCrepe('');
    const parse = parserOf(crepe);
    const base = prepareBaseline('```\ncode\n', '```\ncode\n```\n', parse, (md) => md);
    if (!base) throw new Error('no baseline');
    expect(minimalDiff(base, '```\ncode\n```\n\nAfter\n', parse)).toBeNull();
  });
});

/** mulberry32: a small seeded generator, so a failure names its seed. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

describe('property: an edit rewrites its own block and nothing else', () => {
  const names = Object.keys(EDITOR_FIXTURES).sort();
  const MARK = 'Zq';

  type Edit = 'type' | 'delete' | 'insert';

  /** Applies a random edit; returns its kind, or null when the document offers no place for it. */
  function randomEdit(crepe: Crepe, random: () => number): Edit | null {
    const view = viewOf(crepe);
    const { doc, schema } = view.state;
    const roll = random();
    if (roll < 0.15 && doc.childCount > 1) {
      const index = Math.floor(random() * doc.childCount);
      let from = 0;
      for (let i = 0; i < index; i++) from += doc.child(i).nodeSize;
      view.dispatch(view.state.tr.delete(from, from + doc.child(index).nodeSize));
      return 'delete';
    }
    if (roll < 0.3) {
      // Not after the empty paragraph the editor keeps at the end: that one
      // prints as `<br />` once it is not last, a second new block.
      const trailingEmpty = doc.lastChild?.type.name === 'paragraph' && doc.lastChild.content.size === 0;
      const index = Math.floor(random() * (doc.childCount + (trailingEmpty ? 0 : 1)));
      let at = 0;
      for (let i = 0; i < index; i++) at += doc.child(i).nodeSize;
      view.dispatch(view.state.tr.insert(at, schema.nodes.paragraph.create(null, schema.text(MARK))));
      return 'insert';
    }
    const spots: number[] = [];
    doc.descendants((node, pos) => {
      if (node.isText && node.text) for (let i = 1; i < node.text.length; i++) spots.push(pos + i);
      return true;
    });
    if (spots.length === 0) return null;
    const at = spots[Math.floor(random() * spots.length)];
    view.dispatch(view.state.tr.setSelection(TextSelection.create(doc, at)).insertText(MARK, at));
    return 'type';
  }

  it('over the fixture corpus: random typing, deleting and inserting blocks', async () => {
    let fellBack = 0;
    let runs = 0;
    for (let seed = 1; seed <= 120; seed++) {
      destroyEditors();
      const random = rng(seed);
      const name = names[Math.floor(random() * names.length)];
      const { body: original } = splitFrontmatter(EDITOR_FIXTURES[name].source);
      const crepe = await openCrepe(original);
      const parse = parserOf(crepe);
      const remark = crepe.editor.action((ctx) => ctx.get(remarkCtx));
      const key = (md: string) =>
        JSON.stringify(remark.runSync(remark.parse(md), md), (k, v: unknown) => (k === 'position' || k === 'mbrStyle' ? undefined : v));
      // The minimal path itself, whether or not this fixture's round trip is exact.
      const base = prepareBaseline(original, serializeBody(crepe), parse, key);
      if (!base) throw new Error(`seed ${seed}: ${name} does not split`);
      const edit = randomEdit(crepe, random);
      if (!edit) continue;
      runs++;
      const current = serializeBody(crepe);
      const body = minimalDiff(base, current, parse);
      const label = `seed ${seed} (${name}, ${edit})`;
      if (body === null) {
        fellBack++;
        continue;
      }
      const before = splitBlocks(original, parse);
      const after = splitBlocks(body, parse);
      if (!before || !after) throw new Error(label);
      const kept = matchSequences(before.blocks, after.blocks);
      // Every block but the one edited (or deleted, or inserted) is byte for
      // byte where it was. A deletion may also join its two neighbours (two
      // definition lists with nothing between them are one list), which the
      // save then writes as the one new block they became.
      expect(before.blocks.length - kept.length, label).toBeLessThanOrEqual(edit === 'delete' ? 3 : 1);
      expect(after.blocks.length - kept.length, label).toBeLessThanOrEqual(1);
      if (edit !== 'delete') {
        expect(body.split(MARK).length - 1, label).toBe(1);
        const edited = after.blocks.findIndex((block) => block.includes(MARK));
        expect(kept.some(([, j]) => j === edited), label).toBe(false);
      }
      // And the save means what the whole serialization means.
      expect(key(body), label).toBe(key(current));
    }
    expect(runs).toBeGreaterThan(100);
    expect(fellBack).toBe(0);
  }, 60_000);
});
