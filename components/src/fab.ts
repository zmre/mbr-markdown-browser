/**
 * The floating "play" button shared by `<mbr-slides>` and `<mbr-flashcards>`
 * (main bundle).
 *
 * On a phone the nav's "Play Slides (P)" button costs the page title most of
 * its room, and in the iOS app the web header is collapsed altogether (the
 * native bar replaces it), so neither place has a usable start button. Both
 * triggers therefore also render a round button pinned to the bottom-right
 * corner, shown at `PLAY_FAB_QUERY` widths in any browser and at every width
 * inside the app (an iPad included). Desktop keeps the nav button only.
 *
 * The button is `position: fixed` inside a trigger that lives in the header.
 * That still paints in the app, whose header is collapsed with `height: 0;
 * overflow: hidden` rather than `display: none` (an overflow clip does not
 * apply to fixed descendants).
 *
 * `<mbr-review>`'s own floating button sits in the same corner, so while a
 * play button is showing the controller publishes `--mbr-fab-stack-offset` on
 * `<html>` and the review button stacks above it. Two triggers can be on one
 * page, so the property is reference counted *in the DOM* (`data-mbr-fab-stack`)
 * rather than in module state: one host hiding its button must not pull the
 * offset out from under the other's.
 *
 * Deliberately does not import `app-bridge.ts` for the embedded check: that
 * module imports `mbr-flashcards.ts`, which imports this one.
 */
import { css, type ReactiveController, type ReactiveControllerHost } from 'lit'

/** Below this width the play button floats instead of sitting in the nav. */
export const PLAY_FAB_QUERY = '(max-width: 600px)'

/** How far the review button moves up while a play button is showing. */
export const FAB_STACK_OFFSET = '3.75rem'

const STACK_PROPERTY = '--mbr-fab-stack-offset'
const STACK_COUNT = 'mbrFabStack'

/** True inside the iOS app, which marks `<html>` at document start. */
export function isEmbeddedPage(): boolean {
  return document.documentElement.classList.contains('mbr-embedded')
}

/**
 * The round button, plus hiding the wide nav button where the round one takes
 * over. Shown in the app at every width, but there the nav button is inside a
 * collapsed header already.
 */
export const FAB_CSS = css`
  .mbr-fab {
    position: fixed;
    z-index: 800;
    right: calc(1rem + env(safe-area-inset-right, 0px));
    bottom: calc(1rem + env(safe-area-inset-bottom, 0px));
    width: 3rem;
    height: 3rem;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: var(--pico-primary, #0172ad);
    color: var(--pico-primary-inverse, #fff);
    box-shadow: 0 8px 20px -6px rgba(0, 0, 0, 0.45);
    cursor: pointer;
  }

  .mbr-fab:focus-visible {
    outline: 3px solid var(--pico-primary-focus, rgba(1, 114, 173, 0.5));
    outline-offset: 2px;
  }

  .mbr-fab .play-icon {
    border-left-width: 14px;
    border-top-width: 9px;
    border-bottom-width: 9px;
    margin: 0 0 0 4px;
  }

  @media (max-width: 600px) {
    .play-slides-btn {
      display: none;
    }
  }

  @media print {
    .mbr-fab {
      display: none;
    }
  }
`

/**
 * Tracks whether a floating play button may show (narrow viewport or the iOS
 * app) and keeps `--mbr-fab-stack-offset` in step with whether this host is
 * actually showing one, which `shows` answers after each update.
 */
export class PlayFabController implements ReactiveController {
  private _narrow = false
  private _query: MediaQueryList | null = null
  private _holding = false

  constructor(
    private readonly _host: ReactiveControllerHost,
    private readonly _shows: () => boolean
  ) {
    _host.addController(this)
  }

  /** Whether the floating form applies here, independent of the page. */
  public get visible(): boolean {
    return this._narrow || isEmbeddedPage()
  }

  public hostConnected(): void {
    this._query = window.matchMedia?.(PLAY_FAB_QUERY) ?? null
    this._narrow = this._query?.matches ?? false
    this._query?.addEventListener?.('change', this._onChange)
  }

  public hostUpdated(): void {
    this._hold(this._shows())
  }

  public hostDisconnected(): void {
    this._query?.removeEventListener?.('change', this._onChange)
    this._query = null
    this._hold(false)
  }

  private _onChange = (e: MediaQueryListEvent): void => {
    this._narrow = e.matches
    this._host.requestUpdate()
  }

  /** Take or release this host's share of the stack offset. */
  private _hold(want: boolean): void {
    if (want === this._holding) return
    this._holding = want
    const root = document.documentElement
    const count = Number(root.dataset[STACK_COUNT] ?? 0) + (want ? 1 : -1)
    if (count > 0) {
      root.dataset[STACK_COUNT] = String(count)
      root.style.setProperty(STACK_PROPERTY, FAB_STACK_OFFSET)
    } else {
      delete root.dataset[STACK_COUNT]
      root.style.removeProperty(STACK_PROPERTY)
    }
  }
}
