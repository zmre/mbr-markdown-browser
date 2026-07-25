/**
 * Pure helpers for the editor's heavy-media DOM proxy.
 *
 * mbr authors videos, audio, and PDFs with image syntax — `![caption](/x.mp4)`.
 * In Crepe those render as `<img src="/x.mp4">`, so the browser tries to DOWNLOAD
 * the whole media file as an image before failing; a page with many embeds
 * stalls the editor for a long time.
 *
 * Crepe's ImageBlock `proxyDomURL` sets ONLY the reactive `src` that drives the
 * displayed `<img>` — `node.attrs.src` (what the markdown serializer writes) is
 * left untouched. So swapping heavy media for a lightweight inline-SVG icon here
 * stops the download WITHOUT changing the saved markdown. Real images (and
 * anything we don't recognize) pass through unchanged so image rendering is
 * never broken.
 *
 * Deliberately free of any runtime import so this logic stays cheap and
 * unit-testable. Wired into the Crepe config in editor-crepe.ts.
 */

/** Heavy media that must not be fetched as an image. */
export type HeavyMediaKind = 'video' | 'audio' | 'pdf';

// Extension sets mirror the Rust detection (`../src/vid.rs`, `../src/audio.rs`,
// `../src/media.rs`) and are checked in this priority order. Video wins over
// audio, so `.ogg` classifies as 'video' (matching vid.rs); `.webm` — absent
// from the video set — classifies as 'audio'.
const VIDEO_EXTENSIONS: ReadonlySet<string> = new Set([
  'mp4',
  'mpg',
  'avi',
  'ogv',
  'ogg',
  'm4v',
  'mkv',
  'mov',
]);
const AUDIO_EXTENSIONS: ReadonlySet<string> = new Set([
  'mp3',
  'wav',
  'flac',
  'aac',
  'm4a',
  'webm',
]);

/**
 * The lowercased file extension of `url`, or `null` when there is none. Strips a
 * trailing `?query`/`#fragment` first, then takes the segment after the final
 * `.` — but only when it's a plain alphanumeric run (mirrors the Rust
 * `EXTENSION_RE`), so `data:`/`blob:` payloads and extensionless paths yield
 * `null`.
 */
function extensionOf(url: string): string | null {
  const clean = url.split(/[?#]/, 1)[0] ?? '';
  const dot = clean.lastIndexOf('.');
  if (dot < 0) return null;
  const ext = clean.slice(dot + 1).toLowerCase();
  return /^[0-9a-z]+$/.test(ext) ? ext : null;
}

/**
 * Classifies `url` as heavy media by its extension, or `null` for anything else
 * (real images, `data:`/`blob:`, extensionless/remote URLs) — which must pass
 * through unchanged. Video is checked before audio before PDF (see the
 * extension-set comment for the `.ogg`/`.webm` priority).
 */
export function heavyMediaKind(url: string): HeavyMediaKind | null {
  const ext = extensionOf(url);
  if (!ext) return null;
  if (VIDEO_EXTENSIONS.has(ext)) return 'video';
  if (AUDIO_EXTENSIONS.has(ext)) return 'audio';
  if (ext === 'pdf') return 'pdf';
  return null;
}

/** Wraps an inline SVG body as a `data:image/svg+xml,…` URI (no network). */
const svgIcon = (body: string): string =>
  `data:image/svg+xml,${encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 24 24" fill="none" stroke="#888" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`,
  )}`;

/**
 * One small, recognizable inline-SVG placeholder per heavy-media kind, shown in
 * the editor in place of the un-fetchable media. A mid-gray (`#888`) reads on
 * both light and dark surfaces (a `data:` `<img>` can't inherit the page color).
 */
export const MEDIA_ICON: Readonly<Record<HeavyMediaKind, string>> = {
  // Film frame with a play triangle.
  video: svgIcon(
    '<rect x="3" y="5" width="18" height="14" rx="2"/>' +
      '<path d="M10 9l5 3-5 3z" fill="#888" stroke="none"/>' +
      '<text x="12" y="22.5" font-size="3" fill="#888" stroke="none" text-anchor="middle" font-family="sans-serif">VIDEO</text>',
  ),
  // Twin musical notes.
  audio: svgIcon(
    '<path d="M9 17V5l10-2v12"/>' +
      '<circle cx="7" cy="17" r="2" fill="#888" stroke="none"/>' +
      '<circle cx="17" cy="15" r="2" fill="#888" stroke="none"/>' +
      '<text x="12" y="22.5" font-size="3" fill="#888" stroke="none" text-anchor="middle" font-family="sans-serif">AUDIO</text>',
  ),
  // Document page with a folded corner and a PDF label.
  pdf: svgIcon(
    '<path d="M6 2h8l4 4v16H6z"/>' +
      '<path d="M14 2v4h4"/>' +
      '<text x="12" y="16" font-size="5" fill="#888" stroke="none" text-anchor="middle" font-family="sans-serif">PDF</text>',
  ),
};

/**
 * Crepe `proxyDomURL`: returns a lightweight icon `data:` URI for heavy media so
 * the browser never downloads it, or the original `url` unchanged for everything
 * else. Only affects the displayed `<img>`, never the saved markdown.
 */
export function mediaProxyURL(url: string): string {
  const k = heavyMediaKind(url);
  return k ? MEDIA_ICON[k] : url;
}
