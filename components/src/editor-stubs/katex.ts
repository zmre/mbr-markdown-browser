// Build-time stub for `katex`, aliased in only for the editor builds (see
// vite.editor.config.ts, which vite.markdown-field.config.ts extends).
//
// Crepe statically imports KaTeX for one feature: LaTeX (its inline math node's
// `toDOM` and the code-block preview). `CREPE_FEATURES` (editor-core.ts) turns
// that feature off, and mbr's own math nodes (`editor-plugins/math.ts`) draw
// their source, not KaTeX output, so nothing in either bundle calls it.
// Throwing rather than returning nothing makes a future caller loud.
function notBundled(): never {
  throw new Error("KaTeX is not bundled with mbr's editor: Crepe's LaTeX feature is off")
}

export default { render: notBundled, renderToString: notBundled }
