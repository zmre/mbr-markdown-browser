/**
 * Shared media-failure plumbing.
 *
 * A `<video>` can fail *after* the server happily served the file — WebKit, for
 * example, reports `MEDIA_ERR_DECODE` on MP4s carrying GoPro `gpmd`
 * timed-metadata tracks. Only the browser knows that happened, so this module
 * owns the three pieces both the caption UI (`mbr-video-extras`) and the
 * standalone viewer (`mbr-media-viewer`) need:
 *
 * 1. Turning a `MediaError` code into plain-English, actionable prose.
 * 2. The document-level event channel that lets the page-errors drawer
 *    (`mbr-page-errors`) learn about runtime failures, and lets media elements
 *    learn about the server's richer diagnosis of the same file.
 * 3. One canonical src normalization, so "matching by src" means the same thing
 *    everywhere (relative vs absolute, encoded vs decoded).
 *
 * Everything here is framework-light and pure apart from the small
 * last-page-errors cache, which exists so an element that connects *after*
 * `errors.json` resolved still finds its diagnosis. This module lives in the
 * main bundle only (no lazy chunk imports it).
 */

import { html, css, nothing, type CSSResult, type TemplateResult } from 'lit';

/** Media kinds used by `errors.json`; mirrors `MediaKind` in `src/page_errors.rs`. */
export type MediaKind = 'image' | 'video' | 'audio' | 'source';

/**
 * Server-side *hint* about a likely cause: the file exists and serves fine, but
 * its track layout matches a combination implicated in WebKit decode failures
 * (a `gpmd` timed-metadata track alongside a `tx3g` subtitle track). Mirrors the
 * `unplayable_media` variant of the Rust `PageError` union.
 *
 * This is emphatically NOT a verdict. The combination is necessary but not
 * sufficient — a minimal synthetic file carrying both tracks plays fine in the
 * same Safari build — so an entry with `advisory: true` must never surface on
 * its own. It is shown only to explain a failure this browser has actually
 * reported for the same src, which is why {@link RuntimeMediaError} is the
 * signal that gates it.
 */
export interface UnplayableMediaError {
  type: 'unplayable_media';
  src: string;
  kind: MediaKind;
  reason: string;
  remedy?: string;
  /**
   * `true` when the entry is a heuristic hint rather than a confirmed problem.
   * Absent in older payloads, which are treated as advisory too — the safe
   * default, since the server has never had a way to prove a file won't play.
   */
  advisory?: boolean;
}

/**
 * Client-side observation: this browser actually failed to play the media.
 * Never present in `errors.json` — it is synthesized from a `MediaError`.
 */
export interface RuntimeMediaError {
  type: 'runtime_media_error';
  src: string;
  kind: MediaKind;
  code: number;
  message: string;
}

/**
 * Structural view of a page-error entry. Keeps this module independent of the
 * full `PageError` union (which lives with the element that renders it) while
 * still allowing safe narrowing through {@link isUnplayableMediaError}.
 */
export interface PageErrorLike {
  type: string;
}

/** Dispatched by a media element when its `<video>`/`<audio>` fires `error`. */
export const MEDIA_ERROR_EVENT = 'mbr-media-error';

/** Dispatched by `<mbr-page-errors>` once `errors.json` has been loaded. */
export const PAGE_ERRORS_LOADED_EVENT = 'mbr-page-errors-loaded';

export interface MediaErrorEventDetail {
  src: string;
  kind: MediaKind;
  code: number;
  message: string;
}

export interface PageErrorsLoadedEventDetail {
  errors: readonly PageErrorLike[];
}

declare global {
  interface DocumentEventMap {
    'mbr-media-error': CustomEvent<MediaErrorEventDetail>;
    'mbr-page-errors-loaded': CustomEvent<PageErrorsLoadedEventDetail>;
  }
}

/**
 * Canonical form of a media src, used for all cross-component matching.
 *
 * Resolves relative paths against the current page, drops query/fragment, and
 * percent-decodes, so `../Foo%20Bar.mp4`, `/videos/Foo Bar.mp4` and
 * `http://host/videos/Foo%20Bar.mp4` all compare equal.
 */
export function normalizeMediaSrc(src: string): string {
  try {
    const url = new URL(src, window.location.href);
    return decodeURIComponent(url.pathname);
  } catch {
    // Malformed URL: fall back to the bare path, still trying to decode.
    const bare = src.split(/[?#]/)[0];
    try {
      return decodeURIComponent(bare);
    } catch {
      return bare;
    }
  }
}

/** True when two media srcs point at the same resource. */
export function sameMediaSrc(a: string, b: string): boolean {
  return normalizeMediaSrc(a) === normalizeMediaSrc(b);
}

/** Human-facing description of a media failure. */
export interface MediaErrorNotice {
  /** One-sentence, plain-English explanation. Always present. */
  headline: string;
  /** Secondary detail (server reason, or the browser's own message). */
  detail?: string;
  /** Shell command that fixes the file, when the server knows one. */
  remedy?: string;
}

const MEDIA_ERR_ABORTED = 1;
const MEDIA_ERR_NETWORK = 2;
const MEDIA_ERR_DECODE = 3;
const MEDIA_ERR_SRC_NOT_SUPPORTED = 4;

/** The noun to use when talking about a failed media element. */
function nounFor(kind: MediaKind): string {
  return kind === 'audio' ? 'audio file' : 'video';
}

/**
 * Map a `MediaError.code` to plain English. Deliberately avoids the raw enum
 * names — the reader wants to know what to do, not what WebKit calls it.
 *
 * `message` is the browser's own text; it is surfaced as secondary detail only
 * when it adds something beyond the headline.
 */
export function describeMediaError(
  code: number | null | undefined,
  message?: string | null,
  kind: MediaKind = 'video'
): MediaErrorNotice {
  const noun = nounFor(kind);
  let headline: string;

  switch (code) {
    case MEDIA_ERR_ABORTED:
      headline = `Playback of this ${noun} was aborted before it finished loading.`;
      break;
    case MEDIA_ERR_NETWORK:
      headline = `A network error interrupted the download of this ${noun}. Check the connection and reload the page.`;
      break;
    case MEDIA_ERR_DECODE:
      headline = `This browser could not decode this ${noun}. The file may use a codec, or contain extra tracks, that this browser cannot handle.`;
      break;
    case MEDIA_ERR_SRC_NOT_SUPPORTED:
      headline = `This ${noun}'s format or MIME type is not supported here, or the file is missing.`;
      break;
    default:
      headline = `This ${noun} could not be played.`;
      break;
  }

  const trimmed = (message ?? '').trim();
  const adds = trimmed.length > 0 && !headline.toLowerCase().includes(trimmed.toLowerCase());

  return adds ? { headline, detail: trimmed } : { headline };
}

/**
 * Build the notice to show next to a failed media element.
 *
 * When the server has already diagnosed this exact file, its reason and remedy
 * replace the generic code-based text: the reader learns *why* it failed and
 * how to fix it, instead of just *that* it failed.
 */
export function buildMediaErrorNotice(input: {
  code: number | null | undefined;
  message?: string | null;
  kind?: MediaKind;
  diagnosis?: UnplayableMediaError | null;
}): MediaErrorNotice {
  const kind = input.kind ?? 'video';
  const diagnosis = input.diagnosis;

  if (diagnosis) {
    return {
      headline: `This ${nounFor(diagnosis.kind ?? kind)} cannot be played in this browser.`,
      detail: diagnosis.reason,
      remedy: diagnosis.remedy,
    };
  }

  return describeMediaError(input.code, input.message, kind);
}

/** Type guard narrowing a page-error entry to the `unplayable_media` variant. */
export function isUnplayableMediaError(entry: PageErrorLike): entry is UnplayableMediaError {
  return entry.type === 'unplayable_media';
}

/** HLS playlist MIME type, used for the native-support probe. */
export const HLS_MIME = 'application/vnd.apple.mpegurl';

/**
 * URL of the server's remuxed ("copy") HLS variant of a media file.
 *
 * mbr can repair some containers at serve time by stream-copying video and
 * audio into fMP4 segments, which drops the extra tracks implicated in WebKit
 * decode failures. No re-encode, so no quality loss.
 *
 * Any `#t=` fragment is preserved, since it addresses playback position rather
 * than the resource.
 */
export function remuxUrlFor(src: string): string {
  const hash = src.indexOf('#');
  const base = hash === -1 ? src : src.slice(0, hash);
  const fragment = hash === -1 ? '' : src.slice(hash);
  return `${base}-remux.m3u8${fragment}`;
}

/**
 * Whether this browser can play an HLS playlist from a plain `src`.
 *
 * Only WebKit/Safari has native HLS, which is also where the decode failures we
 * are recovering from occur — so the two line up. Chrome and Firefox return ''
 * here and simply skip recovery rather than swapping to a URL they cannot play.
 */
export function supportsNativeHls(video: HTMLMediaElement): boolean {
  return typeof video.canPlayType === 'function' && video.canPlayType(HLS_MIME) !== '';
}

/**
 * Whether a failure is worth retrying against the remuxed variant.
 *
 * `MEDIA_ERR_DECODE` (3) is the signature of the container problems a remux
 * fixes. `MEDIA_ERR_SRC_NOT_SUPPORTED` (4) is included because WebKit reports
 * some unsupported track layouts that way rather than as a decode error. A
 * network error or an abort says nothing about the container, so those are left
 * alone — retrying them would just fail twice and delay the message.
 */
export function isRecoverableMediaError(code: number | undefined): boolean {
  return code === 3 || code === 4;
}

/**
 * Last successfully loaded `errors.json` payload.
 *
 * The two events can fire in either order; a media element that connects (or
 * fails) *after* the drawer loaded still needs the diagnosis, so the payload is
 * remembered here rather than only pushed out as an event.
 */
let latestPageErrors: readonly PageErrorLike[] = [];

/**
 * Cache the loaded page errors and announce them to media elements.
 * Called by `<mbr-page-errors>` after a successful `errors.json` fetch.
 */
export function publishPageErrors(errors: readonly PageErrorLike[]): void {
  latestPageErrors = errors;
  document.dispatchEvent(
    new CustomEvent<PageErrorsLoadedEventDetail>(PAGE_ERRORS_LOADED_EVENT, {
      detail: { errors },
    })
  );
}

/** Test seam: forget any previously published page errors. */
export function clearPublishedPageErrors(): void {
  latestPageErrors = [];
}

/**
 * Find the server's `unplayable_media` diagnosis for a src, if any.
 * Defaults to the most recently published `errors.json` payload.
 */
export function findUnplayableMedia(
  src: string,
  errors: readonly PageErrorLike[] = latestPageErrors
): UnplayableMediaError | null {
  if (!src) return null;
  const wanted = normalizeMediaSrc(src);
  for (const entry of errors) {
    if (isUnplayableMediaError(entry) && normalizeMediaSrc(entry.src) === wanted) {
      return entry;
    }
  }
  return null;
}

/**
 * Report a media failure to the rest of the page (currently the page-errors
 * drawer). Returns the dispatched detail so callers can reuse it.
 */
export function reportMediaError(
  src: string,
  kind: MediaKind,
  error: MediaError | null | undefined
): MediaErrorEventDetail {
  const detail: MediaErrorEventDetail = {
    src,
    kind,
    code: error?.code ?? 0,
    message: error?.message ?? '',
  };
  document.dispatchEvent(
    new CustomEvent<MediaErrorEventDetail>(MEDIA_ERROR_EVENT, { detail })
  );
  return detail;
}

/**
 * Fold client-observed failures into the server's error list.
 *
 * Two rules, both driven by the fact that only the browser can actually know a
 * file failed to play:
 *
 * 1. An **advisory** `unplayable_media` hint is withheld until a runtime error
 *    has been observed for the same src. Without that, the hint would put a
 *    warning on a video that plays fine — the heuristic has a known false
 *    positive, so this is a real risk rather than a theoretical one.
 * 2. Once a hint *is* shown, the matching runtime error is dropped as a
 *    duplicate: the server entry carries the likely cause and the remedy, the
 *    runtime one only carries an error code.
 *
 * Everything is deduped by canonical src, since a failing `<video>` can fire
 * `error` repeatedly.
 */
export function mergeRuntimeMediaErrors<T extends PageErrorLike>(
  serverErrors: readonly T[],
  runtimeErrors: readonly RuntimeMediaError[]
): (T | RuntimeMediaError)[] {
  const failed = new Set(runtimeErrors.map((entry) => normalizeMediaSrc(entry.src)));

  const keptServer = serverErrors.filter((entry) => {
    if (!isUnplayableMediaError(entry)) return true;
    // `advisory: false` would mean the server is certain; anything else
    // (including an absent field) is treated as a hint.
    if (entry.advisory === false) return true;
    return failed.has(normalizeMediaSrc(entry.src));
  });

  const explained = new Set<string>();
  for (const entry of keptServer) {
    if (isUnplayableMediaError(entry)) {
      explained.add(normalizeMediaSrc(entry.src));
    }
  }

  const seen = new Set<string>();
  const kept: RuntimeMediaError[] = [];
  for (const entry of runtimeErrors) {
    const key = normalizeMediaSrc(entry.src);
    if (explained.has(key) || seen.has(key)) continue;
    seen.add(key);
    kept.push(entry);
  }

  return [...keptServer, ...kept];
}

/**
 * Styles for {@link renderMediaErrorNotice}. Add to a component's static
 * `styles` array; every colour is a Pico custom property with a fallback, so
 * the notice stays readable in both light and dark themes.
 */
export const mediaErrorStyles: CSSResult = css`
  .media-error {
    display: flex;
    gap: 0.6em;
    align-items: flex-start;
    margin: 0.5em 0;
    padding: 0.6em 0.75em;
    border: 1px solid var(--pico-form-element-invalid-border-color, #96494f);
    border-left-width: 4px;
    border-radius: 4px;
    background: var(--pico-code-background-color, rgba(127, 127, 127, 0.12));
    color: var(--pico-color, inherit);
    /* rem, not em: the caption host shrinks its text, and an error message
       still has to be comfortably readable. */
    font-size: 0.95rem;
    line-height: 1.45;
    text-align: left;
  }

  .media-error-icon {
    flex: 0 0 auto;
    color: var(--pico-del-color, #883835);
    font-size: 1.1em;
    line-height: 1.3;
  }

  .media-error-body {
    flex: 1 1 auto;
    min-width: 0;
  }

  .media-error-headline {
    margin: 0;
    color: var(--pico-del-color, #883835);
    font-weight: 600;
  }

  .media-error-detail,
  .media-error-remedy {
    margin: 0.35em 0 0 0;
    color: var(--pico-muted-color, #666);
  }

  .media-error-remedy code {
    display: inline-block;
    max-width: 100%;
    padding: 0.15em 0.4em;
    border-radius: 3px;
    background: var(--pico-background-color, #fff);
    color: var(--pico-code-color, inherit);
    font-family: var(--pico-font-family-monospace, ui-monospace, SFMono-Regular, Menlo, monospace);
    font-size: 0.9em;
    overflow-wrap: anywhere;
    user-select: all;
  }
`;

/**
 * Render a media failure as a caption-sized alert. Announced to assistive tech
 * via `role="alert"` because it appears in response to a user action (play).
 */
export function renderMediaErrorNotice(notice: MediaErrorNotice): TemplateResult {
  return html`
    <div class="media-error" role="alert">
      <span class="media-error-icon" aria-hidden="true">&#9888;</span>
      <div class="media-error-body">
        <p class="media-error-headline">${notice.headline}</p>
        ${notice.detail
          ? html`<p class="media-error-detail">${notice.detail}</p>`
          : nothing}
        ${notice.remedy
          ? html`<p class="media-error-remedy">
              Re-encode with: <code>${notice.remedy}</code>
            </p>`
          : nothing}
      </div>
    </div>
  `;
}
