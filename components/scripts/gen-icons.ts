#!/usr/bin/env bun
/**
 * Regenerates `src/icon-shapes.ts` from `crates/mbr-core/templates/icons/*.svg`.
 *
 *     cd components
 *     bun scripts/gen-icons.ts
 *
 * The output is checked in because the Nix components build sees only
 * `components/`. See `src/icon-codegen.ts` for the format.
 */
import { readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { renderIconShapesModule } from '../src/icon-codegen.ts'

const dir = fileURLToPath(new URL('../../crates/mbr-core/templates/icons', import.meta.url))
const out = fileURLToPath(new URL('../src/icon-shapes.ts', import.meta.url))
const files = Object.fromEntries(
  readdirSync(dir)
    .filter((f) => f.endsWith('.svg'))
    .map((f) => [f, readFileSync(`${dir}/${f}`, 'utf8')])
)
writeFileSync(out, renderIconShapesModule(files))
console.log(`wrote ${out}`)
