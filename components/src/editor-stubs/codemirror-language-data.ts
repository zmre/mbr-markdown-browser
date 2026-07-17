// Build-time stub for `@codemirror/language-data`, aliased in only for the
// editor chunk (see vite.editor.config.ts).
//
// Milkdown/Crepe statically imports `languages` from this package and stashes it
// in its default CodeMirror config. The real package's `languages` array holds
// ~50 `() => import('@codemirror/lang-*')` thunks — a large chunk of the bundle
// and, when inlined into one file, a source of module init-order breakage.
//
// The editor disables the CodeMirror feature (editor-crepe.ts), so those
// language modules are never loaded and `languages` is never read. Replacing it
// with an empty list keeps the whole graph out of the bundle.
export const languages: unknown[] = [];
