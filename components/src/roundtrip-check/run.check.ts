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
  const minimizeMs = Number(process.env.MBR_ROUNDTRIP_MINIMIZE_MS ?? '2000');
  const samples = Number(process.env.MBR_ROUNDTRIP_SAMPLES ?? '');
  const totalMs = Number(process.env.MBR_ROUNDTRIP_MINIMIZE_TOTAL_MS ?? '');
  let written = 0;
  await runRoundTripCheck({
    root,
    out,
    minimizeMs: Number.isFinite(minimizeMs) ? minimizeMs : 2000,
    samplesPerCause: Number.isInteger(samples) && samples >= 0 ? samples : undefined,
    minimizeTotalMs: Number.isFinite(totalMs) && totalMs >= 0 && process.env.MBR_ROUNDTRIP_MINIMIZE_TOTAL_MS ? totalMs : undefined,
    onProgress: (done, total) => {
      if (done < total && Date.now() - written < 250) return;
      written = Date.now();
      writeFileSync(join(out, 'progress.json'), JSON.stringify({ done, total }));
    },
  });
});
