/**
 * A CRLF file is saved back with CRLF line endings, frontmatter included
 * (`lineEndingOf` / `withLineEnding`, applied by `openEditor` and mirrored by
 * `savedFile`).
 */
import { editorViewCtx } from '@milkdown/kit/core';
import { afterEach, describe, expect, it } from 'vitest';
import { destroyEditors, savedFile } from './editor-test-support.js';

afterEach(destroyEditors);

const CRLF_DOC =
  '---\r\ntitle: CRLF\r\n---\r\n# CRLF file\r\n\r\nA paragraph\r\nwith a soft break.\r\n\r\n* [ ] task\r\n* item\r\n\r\n```\r\ncode line\r\n```\r\n';

describe('line endings', () => {
  it('saves an untouched CRLF file byte for byte', async () => {
    expect(await savedFile(CRLF_DOC)).toBe(CRLF_DOC);
  });

  it('writes CRLF for edited content too', async () => {
    const saved = await savedFile(CRLF_DOC, (crepe) => {
      crepe.editor.action((ctx) => {
        const view = ctx.get(editorViewCtx);
        view.dispatch(view.state.tr.insertText('New ', 1));
      });
    });
    expect(saved).toBe(CRLF_DOC.replace('# CRLF file', '# New CRLF file'));
    expect(saved.replace(/\r\n/g, '')).not.toContain('\n');
  });

  it('leaves an LF file LF', async () => {
    const lf = CRLF_DOC.replace(/\r\n/g, '\n');
    expect(await savedFile(lf)).toBe(lf);
  });
});
