/**
 * Entry of `mbr-export-ready.min.js` (`vite.export-ready.config.ts`): runs
 * `export-ready-standalone.ts` with this script's own URL, read while the
 * script is being evaluated (`document.currentScript` is gone afterwards).
 */
import { bootStandaloneExport } from './export-ready-standalone.ts'

const script = document.currentScript as HTMLScriptElement | null
void bootStandaloneExport(script?.src)
