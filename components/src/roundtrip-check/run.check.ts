/// <reference types="node" />
/**
 * The vitest entry of `roundtrip-check` (it needs the editor, and so a DOM).
 * Not part of `bun run test`: only `vitest.roundtrip.config.ts` includes it,
 * and `cli.ts` runs it with the folder, report and budget in the environment.
 * Its progress goes to `<report>/progress.json` as counts, never to stdout.
 */
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { it } from 'vitest';
import { runRoundTripCheck } from './runner.js';

it('roundtrip-check', async () => {
  const root = process.env.MBR_ROUNDTRIP_DIR;
  const out = process.env.MBR_ROUNDTRIP_OUT;
  if (!root || !out) throw new Error('MBR_ROUNDTRIP_DIR and MBR_ROUNDTRIP_OUT are not set: run `bun run roundtrip-check`');
  const minimizeMs = Number(process.env.MBR_ROUNDTRIP_MINIMIZE_MS ?? '10000');
  let written = 0;
  await runRoundTripCheck({
    root,
    out,
    minimizeMs: Number.isFinite(minimizeMs) ? minimizeMs : 10000,
    onProgress: (done, total) => {
      if (done < total && Date.now() - written < 250) return;
      written = Date.now();
      writeFileSync(join(out, 'progress.json'), JSON.stringify({ done, total }));
    },
  });
});
