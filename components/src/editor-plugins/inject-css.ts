/**
 * Shared by the editor plugins that carry their own stylesheet: inject `css`
 * into `<head>` once per `id`, the first time an editor with the plugin
 * mounts. The editor lives in the page's light DOM (see `injectStyles` in
 * `editor-crepe.ts`), so a document stylesheet is what reaches its nodes and
 * can use the page theme's custom properties.
 */
export function injectEditorCss(id: string, css: string): void {
  if (typeof document === 'undefined' || document.getElementById(id)) return;
  const style = document.createElement('style');
  style.id = id;
  style.textContent = css;
  document.head.appendChild(style);
}
