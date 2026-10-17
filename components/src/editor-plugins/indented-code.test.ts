/**
 * One indented code block where micromark makes two (`indented-code.ts`);
 * the untouched round trip and the outline are the parity harness's
 * `rt-empty-item-indented-code`.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { remarkCtx } from '@milkdown/kit/core';
import { destroyEditors, openCrepe, savedFile } from '../editor-test-support.js';

afterEach(destroyEditors);

interface Block {
  type: string;
  value?: string;
  position?: { start: { line: number }; end: { line: number } };
}

/** The top-level blocks of `source` as the editor's processor reads them (transforms included). */
async function blocks(source: string): Promise<Block[]> {
  const crepe = await openCrepe('');
  const tree = crepe.editor.action((ctx) => {
    const remark = ctx.get(remarkCtx);
    return remark.runSync(remark.parse(source), source);
  }) as unknown as { children: Block[] };
  return tree.children;
}

describe('indented code after an empty list item', () => {
  it('is one block, blank lines and deeper indentation included', async () => {
    const source = '-\n\n    - one\n        - two\n\n    - three\n- next\n';
    const code = (await blocks(source)).filter((block) => block.type === 'code');
    expect(code).toHaveLength(1);
    expect(code[0].value).toBe('- one\n    - two\n\n- three');
    expect([code[0].position?.start.line, code[0].position?.end.line]).toEqual([3, 6]);
  });

  it('round-trips, and stays one block when printed', async () => {
    const source = '- \n\n    - one\n        - two\n- next\n';
    expect(await savedFile(source)).toBe(source);
  });

  it('leaves two code blocks with something between them alone', async () => {
    const source = '    one\n\npara\n\n    two\n';
    expect((await blocks(source)).map((block) => block.type)).toEqual(['code', 'paragraph', 'code']);
  });
});
