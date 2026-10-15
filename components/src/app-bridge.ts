/**
 * The page's half of the iOS app bridge. Inert everywhere but the app.
 *
 * In the app, mbr's own header and breadcrumbs are hidden
 * (`html.mbr-embedded` in theme.css) and the native navigation bar takes over:
 * the page title, a menu of the breadcrumb path, and a ⋯ menu with every
 * action the page supports. This module is what feeds that bar and what the
 * bar calls back into.
 *
 * - **Page → app:** on every page load, {@link postPageState} sends
 *   `{type: 'page', url, title, breadcrumbs, actions}` to the
 *   `webkit.messageHandlers.mbrApp` script-message handler the app installs.
 * - **Page → app, editor:** when the full-screen editor opens or closes
 *   (`mbr-editor.ts`'s {@link EDITOR_STATE_EVENT}), `{type: 'editor', open}`,
 *   so the app can hide its bar and turn off the back/forward swipe while
 *   there are edits to lose.
 * - **Page → app, slides:** when a presentation starts playing or stops
 *   (`mbr-slides.ts`'s {@link SLIDES_STATE_EVENT}), `{type: 'slides', playing}`,
 *   so the app can hide its bar (a landscape deck is squashed under it) and
 *   turn off the back/forward swipe, which Reveal's slide swipes would trip.
 * - **App → page:** `window.mbrApp.open(name)` opens one of
 *   {@link APP_ACTIONS} through the same public method its own button and
 *   shortcut use (`open()`, `openNew()` for `new`, `<mbr-fuzzy-nav>`'s
 *   `open(tab)` for `toc`/`links-out`/`links-in`), so the app never
 *   re-implements a feature.
 *
 * The app marks the page as embedded with a user script injected at document
 * start (`window.__MBR_EMBEDDED__ = true`, `html.mbr-embedded`), before any
 * page script runs; nothing server-side changes. **The action names and
 * `window.mbrApp.open` are referenced from Swift string literals**
 * (`apple/ios/MBRMobile/Web/AppBridge.swift`), so renaming one cannot fail at
 * compile time; `app-bridge.test.ts` pins them.
 *
 * Gating mirrors what the page itself offers: an action is listed only when
 * its element is on the page *and* usable (edit only with editing on and a
 * source file, new only with editing on, tasks only with the task browser enabled, flashcards and slides
 * only on a deck, contents only with headings, links only on a note page), so
 * the app's menu shows exactly what works.
 */
import { isTasksEnabled } from './shared.js';
import { isDeckPage } from './mbr-flashcards.js';
// mbr-editor imports only shared/mbr-keys/edit-token (no cycle) and is in the
// main bundle already; its heavy editor is a lazy chunk.
import { EDITOR_STATE_EVENT } from './mbr-editor.js';
// mbr-slides is already in this graph (mbr-flashcards imports it) and imports
// nothing that comes back here.
import { SLIDES_STATE_EVENT } from './mbr-slides.js';

/** Every action the app can ask for, in the app menu's order. */
export const APP_ACTIONS = [
  'browse',
  'search',
  'edit',
  'info',
  'toc',
  'links-out',
  'links-in',
  'tasks',
  'flashcards',
  'slides',
  'new',
] as const;

export type AppAction = (typeof APP_ACTIONS)[number];

/** The `WKScriptMessageHandler` name the app registers. */
export const APP_MESSAGE_HANDLER = 'mbrApp';

export interface Crumb {
  name: string;
  /** Site-absolute URL path (`/notes/`). */
  url: string;
}

export interface PageState {
  type: 'page';
  url: string;
  title: string;
  /** The path above this page, outermost first; the page itself is `title`. */
  breadcrumbs: Crumb[];
  actions: AppAction[];
  /**
   * Whether the full-screen editor is open. Usually `false`, but the `load`
   * re-post can come after an editor that opened with the page (a note just
   * created opens in it), and the app resets its state on every page message.
   */
  editorOpen: boolean;
  /** Whether a slide presentation is playing; the same reasoning as `editorOpen`. */
  slidesPlaying: boolean;
}

/** The full-screen editor opened or closed. */
export interface EditorState {
  type: 'editor';
  open: boolean;
}

/** A slide presentation started or stopped. */
export interface SlidesState {
  type: 'slides';
  playing: boolean;
}

export interface AppBridge {
  /** Opens `name`; `false` when it is not an action or not available here. */
  open(name: string): boolean;
  /** What {@link postPageState} would send, for the app to ask again. */
  state(): PageState;
  readonly actions: readonly AppAction[];
}

declare global {
  interface Window {
    /** Set by the iOS app's document-start user script. */
    __MBR_EMBEDDED__?: boolean;
    mbrApp?: AppBridge;
    webkit?: { messageHandlers?: Record<string, { postMessage(message: unknown): void } | undefined> };
  }
}

/** Whether this page is shown inside the iOS app. */
export function isEmbedded(): boolean {
  return window.__MBR_EMBEDDED__ === true;
}

type OpenMethod = 'open' | 'openNew';

type Openable = HTMLElement & Record<OpenMethod, ((...args: string[]) => void) | undefined>;

interface ActionTarget {
  selector: string;
  usable: (el: HTMLElement) => boolean;
  /** The element's public method that performs the action; `open` when absent. */
  method?: OpenMethod;
  /** Arguments for that method, when one element serves several actions. */
  args?: readonly string[];
}

/** Whether `<mbr-fuzzy-nav>` reports `gate` (its `hasHeadings`/`hasPageLinks`). */
const fuzzyNav =
  (gate: 'hasHeadings' | 'hasPageLinks') =>
  (el: HTMLElement): boolean =>
    (el as HTMLElement & Partial<Record<typeof gate, boolean>>)[gate] === true;

const always = (): boolean => true;

/**
 * Where each action lives. The selector is the element whose public `open()`
 * (or `method`) is the action; `usable` is the page's own gate for it.
 */
const ACTION_TARGETS: Record<AppAction, ActionTarget> = {
  // `single` sidebar style renders <mbr-browse-single>, the other <mbr-browse>.
  browse: { selector: 'mbr-browse-single, mbr-browse', usable: always },
  search: { selector: 'mbr-search', usable: always },
  edit: { selector: 'mbr-editor', usable: (el) => (el as HTMLElement & { canEdit?: boolean }).canEdit === true },
  info: { selector: 'mbr-info', usable: always },
  // The f / F / T panel, one element on three tabs.
  toc: { selector: 'mbr-fuzzy-nav', usable: fuzzyNav('hasHeadings'), args: ['toc'] },
  'links-out': { selector: 'mbr-fuzzy-nav', usable: fuzzyNav('hasPageLinks'), args: ['links-out'] },
  'links-in': { selector: 'mbr-fuzzy-nav', usable: fuzzyNav('hasPageLinks'), args: ['links-in'] },
  tasks: { selector: 'mbr-tasks', usable: () => isTasksEnabled() },
  flashcards: { selector: 'mbr-flashcards', usable: () => isDeckPage() },
  slides: {
    selector: 'mbr-slides',
    usable: (el) => (el as HTMLElement & { isSlideDocument?: boolean }).isSlideDocument === true,
  },
  new: {
    selector: 'mbr-editor',
    usable: (el) => (el as HTMLElement & { canCreate?: boolean }).canCreate === true,
    method: 'openNew',
  },
};

/** The action's opener, bound to its element, or `null` when unavailable. */
function targetFor(action: AppAction, root: ParentNode): (() => void) | null {
  const { selector, usable, method = 'open', args = [] } = ACTION_TARGETS[action];
  const el = root.querySelector<HTMLElement>(selector) as Openable | null;
  if (!el || typeof el[method] !== 'function' || !usable(el)) return null;
  return () => el[method]?.(...args);
}

function isAppAction(name: string): name is AppAction {
  return (APP_ACTIONS as readonly string[]).includes(name);
}

/** The actions this page supports right now, in menu order. */
export function availableActions(root: ParentNode = document): AppAction[] {
  return APP_ACTIONS.filter((action) => targetFor(action, root) !== null);
}

/** Opens `name` through its element's public `open()` (or `openNew()`), with its tab where it has one. */
export function openAppAction(name: string, root: ParentNode = document): boolean {
  if (!isAppAction(name)) return false;
  const open = targetFor(name, root);
  if (!open) return false;
  open();
  return true;
}

/** The page title as mbr's header shows it. */
export function pageTitle(doc: Document = document): string {
  const nav = doc.querySelector('body > header .mbr-nav-title strong')?.textContent?.trim();
  return nav || doc.title.trim();
}

/**
 * The path above this page. From the rendered breadcrumbs where the page has
 * them (markdown pages), else from the URL's folders (listings, tag pages),
 * `Home` first. The home page has none.
 */
export function pageBreadcrumbs(doc: Document = document, href: string = window.location.href): Crumb[] {
  const links = [...doc.querySelectorAll<HTMLAnchorElement>('nav.breadcrumbs li:not(.mbr-crumbs-more) a[href]')];
  if (links.length > 0) {
    return links.map((a) => ({
      name: a.textContent?.trim() ?? '',
      url: new URL(a.getAttribute('href') ?? '/', href).pathname,
    }));
  }
  const segments = new URL(href).pathname.split('/').filter(Boolean);
  if (segments.length === 0) return [];
  return [
    { name: 'Home', url: '/' },
    ...segments.slice(0, -1).map((segment, i) => ({
      name: decodeURIComponent(segment),
      url: `/${segments.slice(0, i + 1).join('/')}/`,
    })),
  ];
}

export function pageState(doc: Document = document, href: string = window.location.href): PageState {
  return {
    type: 'page',
    url: href,
    title: pageTitle(doc),
    breadcrumbs: pageBreadcrumbs(doc, href),
    actions: availableActions(doc),
    editorOpen,
    slidesPlaying,
  };
}

/** Sends {@link pageState} to the app. `false` outside the app. */
export function postPageState(): boolean {
  const handler = window.webkit?.messageHandlers?.[APP_MESSAGE_HANDLER];
  if (!isEmbedded() || !handler) return false;
  handler.postMessage(pageState());
  return true;
}

/** Sends `{type: 'editor', open}` to the app. `false` outside the app. */
export function postEditorState(open: boolean): boolean {
  const handler = window.webkit?.messageHandlers?.[APP_MESSAGE_HANDLER];
  if (!isEmbedded() || !handler) return false;
  const message: EditorState = { type: 'editor', open };
  handler.postMessage(message);
  return true;
}

/** The last state {@link EDITOR_STATE_EVENT} reported, for {@link pageState}. */
let editorOpen = false;

function onEditorState(event: Event): void {
  const open = (event as CustomEvent<{ open?: unknown }>).detail?.open;
  if (typeof open !== 'boolean') return;
  editorOpen = open;
  postEditorState(open);
}

/** Sends `{type: 'slides', playing}` to the app. `false` outside the app. */
export function postSlidesState(playing: boolean): boolean {
  const handler = window.webkit?.messageHandlers?.[APP_MESSAGE_HANDLER];
  if (!isEmbedded() || !handler) return false;
  const message: SlidesState = { type: 'slides', playing };
  handler.postMessage(message);
  return true;
}

/** The last state {@link SLIDES_STATE_EVENT} reported, for {@link pageState}. */
let slidesPlaying = false;

function onSlidesState(event: Event): void {
  const playing = (event as CustomEvent<{ playing?: unknown }>).detail?.playing;
  if (typeof playing !== 'boolean') return;
  slidesPlaying = playing;
  postSlidesState(playing);
}

/** Installs `window.mbrApp` and reports the page, in the app only. */
export function installAppBridge(): void {
  if (!isEmbedded()) return;
  window.mbrApp = {
    open: (name: string) => openAppAction(name),
    state: () => pageState(),
    actions: APP_ACTIONS,
  };
  // Once the DOM is there (the components are module scripts, so they have
  // upgraded by then), and again at `load`: a deck page's `type: flashcard`
  // check and lazily-defined elements can change what is available.
  const post = (): void => {
    postPageState();
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', post, { once: true });
  } else {
    post();
  }
  window.addEventListener('load', post, { once: true });
  // Replacing, not stacking, if installed twice (tests do).
  document.removeEventListener(EDITOR_STATE_EVENT, onEditorState);
  document.addEventListener(EDITOR_STATE_EVENT, onEditorState);
  document.removeEventListener(SLIDES_STATE_EVENT, onSlidesState);
  document.addEventListener(SLIDES_STATE_EVENT, onSlidesState);
}

if (typeof window !== 'undefined') installAppBridge();
