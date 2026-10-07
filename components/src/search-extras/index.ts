/**
 * Entry point for the lazy `mbr-search-extras.min.js` chunk (built by
 * vite.search-extras.config.ts), loaded by `<mbr-search>` the first time the
 * search modal opens in server/GUI mode. Importing it registers
 * `<mbr-folder-picker>`; the type-list and facet helpers are returned as the
 * module's exports.
 *
 * Nothing here may import `shared.ts` or any other stateful main-bundle module:
 * the site.json payload is handed in by the main bundle.
 */
export { MbrFolderPickerElement, type FolderPickEventDetail } from './mbr-folder-picker.js'
export {
  deriveSearchFacets,
  foldersFromUrlPaths,
  noteTypesFromSite,
  typeFacetToken,
  withTypeFacet,
  type NoteTypeCount,
} from './facets.js'
