/**
 * Entry of the standalone `mbr-markdown-field.min.js` bundle (built by
 * `vite.markdown-field.config.ts`, served at
 * `/.mbr/components/mbr-markdown-field.min.js`): importing it defines
 * `<mbr-markdown-field>`.
 *
 * Self-contained: it shares source with the `mbr-editor.min.js` chunk
 * (`editor-core.ts`) but no runtime state with anything, and a host page can
 * load it without mbr's main bundle.
 */
import { defineMarkdownField } from './mbr-markdown-field.js';

export {
  CHANGE_DEBOUNCE_MS,
  MARKDOWN_FIELD_TAG,
  MbrMarkdownField,
  defineMarkdownField,
  type MarkdownFieldChangeDetail,
  type MarkdownFieldUploader,
} from './mbr-markdown-field.js';

defineMarkdownField();
