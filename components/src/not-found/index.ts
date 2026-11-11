/**
 * Entry of the lazy `mbr-not-found.min.js` chunk: where a missing page
 * probably went. Loaded by `<mbr-not-found-suggestions>` on 404 pages and by
 * `<mbr-page-errors>` when its panel shows broken links, both through
 * `not-found-chunk.ts`.
 *
 * Nothing here may import `shared.ts` (or any stateful main-bundle module):
 * site.json, media.json and `resolveUrl` arrive as arguments.
 */
export {
  didYouMean,
  findMatches,
  folderFallback,
  namesAFile,
  pageExists,
  suggestForMissing,
} from './match.js'
export { renderNotFound } from './render.js'
