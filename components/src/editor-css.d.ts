// Allow importing CSS as an inlined string (Vite `?inline`), so the editor
// chunk can inject Crepe's stylesheet into the document without relying on the
// bundler auto-loading an extracted CSS file (which it does not for lib builds
// consumed via runtime dynamic import).
declare module '*.css?inline' {
  const css: string;
  export default css;
}
