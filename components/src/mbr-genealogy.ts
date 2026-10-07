/**
 * `<mbr-genealogy>` — lightweight trigger for the "Relationships" charts on
 * person and organization pages. (The element and chunk keep their historical
 * genealogy names; renaming them would break users' template overrides.)
 *
 * Lives in the main bundle — which every page pays for — and therefore does as
 * little as possible: it guards on `type: person`/`organization` frontmatter,
 * checks that the focus note has at least one resolved relationship (a scan of
 * that ONE note), renders a fixed-height placeholder (no layout shift) and,
 * once the placeholder nears the viewport (IntersectionObserver, 400px margin),
 * dynamically imports the `mbr-genealogy.min.js` chunk and hands it the
 * site.json notes plus services via `mountGenealogy()`.
 *
 * Building the relationship graph, choosing the opening chart and reporting
 * contradictory links all happen in the chunk: none of it is needed for a chart
 * that never scrolls into view. A note with no resolved relationship to another
 * note renders nothing at all.
 */
import { LitElement, html, css, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { waitForDom, getMbrAssetBase } from './dynamic-loader.ts'
import { subscribeSiteNav, getCanonicalPath, getGraphDepth, resolveUrl } from './shared.ts'
import { loadGraphChunk } from './graph-chunk.ts'
import { fetchPageLinks } from './graph/links-cache.ts'
import {
  DEFAULT_DEPTH,
  DEFAULT_MAX_NODES,
  canonicalizeNotePath,
  notesByPathFromSite,
  type RelationTypeConfig,
  type SiteNote,
} from './graph/relationship-graph.js'
import type { GenealogyController, GenealogyMountInput } from './genealogy/index.js'

/** Note types that get the relationship charts. */
const CHART_NOTE_TYPES = new Set(['person', 'organization'])

/** True when the page's frontmatter `type` is one that gets the charts. */
export function isChartNoteType(type: unknown): boolean {
  return typeof type === 'string' && CHART_NOTE_TYPES.has(type.trim().toLowerCase())
}

/**
 * True when `focus` has at least one resolved relationship to another known
 * note — of ANY type or category. This, not a graph's edge count, is the gate:
 * the family graph drops siblings and a work-only note has no family edges, yet
 * both still have something to chart (the org chart, All people).
 */
export function hasChartableRelationships(focus: string, notesByPath: Map<string, SiteNote>): boolean {
  const note = notesByPath.get(canonicalizeNotePath(focus))
  return (note?.relationships ?? []).some(
    (rel) => rel.resolved && !!rel.neighbor && rel.neighbor !== note?.url_path && notesByPath.has(rel.neighbor)
  )
}

/** Shape of the lazily-loaded chunk (type-only; erased at build). */
type GenealogyModule = {
  mountGenealogy(container: HTMLElement, input: GenealogyMountInput): GenealogyController
}

type GenealogyModuleLoader = (url: string) => Promise<GenealogyModule>

/** Default loader: runtime dynamic import of the separately-built chunk. */
const defaultLoader: GenealogyModuleLoader = (url) =>
  import(/* @vite-ignore */ url) as Promise<GenealogyModule>

let moduleLoader: GenealogyModuleLoader = defaultLoader

/**
 * Test seam: override how the chunk is imported (happy-dom cannot execute
 * runtime URL imports). Pass `null` to restore the default loader.
 */
export function setGenealogyModuleLoader(loader: GenealogyModuleLoader | null): void {
  moduleLoader = loader ?? defaultLoader
}

@customElement('mbr-genealogy')
export class MbrGenealogyElement extends LitElement {
  /** How many relationship hops the family graph expands from the focus. */
  @property({ type: Number })
  depth = DEFAULT_DEPTH

  /** Safety cap on graph size for very large repositories. */
  @property({ type: Number, attribute: 'max-nodes' })
  maxNodes = DEFAULT_MAX_NODES

  /** True once site.json shows the focus note has something to chart. */
  @state()
  private _chartable = false

  @state()
  private _mounted = false

  @state()
  private _failed = false

  private _siteData: { markdown_files?: SiteNote[]; relationship_types?: RelationTypeConfig[] } | null =
    null
  private _notesByPath: Map<string, SiteNote> = new Map()
  private _focus = ''
  private _unsubscribeSiteNav?: () => void
  private _observer: IntersectionObserver | null = null
  private _loadArmed = false
  private _loadPromise: Promise<void> | null = null
  private _module: GenealogyModule | null = null
  private _controller: GenealogyController | null = null

  override connectedCallback() {
    super.connectedCallback()
    void waitForDom().then(() => {
      // Only person and organization pages get the relationship charts.
      if (!isChartNoteType(window.frontmatter?.['type'])) return
      this._unsubscribeSiteNav = subscribeSiteNav((state) => {
        if (state.data && state.data !== this._siteData) {
          this._siteData = state.data
          this._refresh()
        }
      })
    })
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    this._unsubscribeSiteNav?.()
    this._unsubscribeSiteNav = undefined
    this._observer?.disconnect()
    this._observer = null
    this._controller?.destroy()
    this._controller = null
  }

  override updated(changed: PropertyValues) {
    if ((changed.has('depth') || changed.has('maxNodes')) && this._siteData) {
      this._refresh()
    }
    if (this._hasChart() && !this._loadArmed) {
      this._armLoad()
    }
  }

  private _hasChart(): boolean {
    return !this._failed && this._chartable
  }

  /** Re-read site.json data; remount an already-mounted chart with it. */
  private _refresh(): void {
    const data = this._siteData
    if (!data) return
    this._notesByPath = notesByPathFromSite(data)
    this._focus = canonicalizeNotePath(getCanonicalPath())
    this._chartable = hasChartableRelationships(this._focus, this._notesByPath)
    if (this._controller) {
      this._controller.destroy()
      this._controller = null
      this._mounted = false
      if (this._hasChart()) void this._mountChart()
    }
  }

  /**
   * Arm the lazy load: wait until the placeholder is within 400px of the
   * viewport. When IntersectionObserver is unavailable (older browsers,
   * happy-dom in tests), load immediately.
   */
  private _armLoad(): void {
    this._loadArmed = true
    if (typeof IntersectionObserver === 'undefined') {
      void this._load()
      return
    }
    this._observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          this._observer?.disconnect()
          this._observer = null
          void this._load()
        }
      },
      { rootMargin: '400px' }
    )
    this._observer.observe(this)
  }

  private _load(): Promise<void> {
    this._loadPromise ??= (async () => {
      try {
        const url = new URL(
          `${getMbrAssetBase()}components/mbr-genealogy.min.js`,
          document.baseURI
        ).href
        this._module = await moduleLoader(url)
        await this._mountChart()
      } catch (err) {
        console.warn('[mbr-genealogy] Failed to load the relationship chart chunk:', err)
        this._failed = true
      }
    })()
    return this._loadPromise
  }

  private async _mountChart(): Promise<void> {
    if (!this._module || !this._chartable || this._controller) return
    // Make sure the mount container from the current template is in the DOM.
    await this.updateComplete
    const container = this.shadowRoot?.querySelector<HTMLElement>('.gen-mount')
    if (!container) return
    const types = this._siteData?.relationship_types
    this._controller = this._module.mountGenealogy(container, {
      notesByPath: this._notesByPath,
      relationshipTypes: Array.isArray(types) ? types : [],
      focusPath: this._focus,
      depth: this.depth,
      maxNodes: this.maxNodes,
      resolveUrl,
      navigate: (path: string) => window.location.assign(resolveUrl(path)),
      graphDepth: getGraphDepth(),
      loadGraphChunk,
      fetchPageLinks,
    })
    this._mounted = true
  }

  override render() {
    if (!this._hasChart()) return nothing
    return html`
      <figure class="gen-figure" role="group" aria-label="Relationship charts">
        <figcaption>Relationships</figcaption>
        <div class="gen-canvas">
          <div class="gen-mount"></div>
          ${this._mounted
            ? nothing
            : html`
                <div class="gen-loading" role="status" aria-label="Loading relationship chart">
                  <span class="gen-spinner" aria-hidden="true"></span>
                </div>
              `}
        </div>
      </figure>
    `
  }

  static override styles = css`
    :host {
      display: block;
    }

    .gen-figure {
      max-width: 1024px;
      margin: 2rem auto;
      padding: 1rem 1.25rem 1.25rem;
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      border-radius: 8px;
      background: var(--pico-card-background-color, transparent);
    }

    .gen-figure figcaption {
      font-weight: 600;
      margin-bottom: 0.75rem;
      color: var(--pico-color, #333);
    }

    /* Fixed-height chart window reserved up front, so the lazy chunk causes no
       layout shift when it mounts (the contradictory-link notice, if any, is
       drawn inside it by the chunk). */
    .gen-canvas {
      position: relative;
      height: min(70vh, 640px);
      overflow: hidden;
      border-radius: 4px;
    }

    .gen-mount {
      height: 100%;
    }

    .gen-loading {
      position: absolute;
      inset: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      pointer-events: none;
    }

    .gen-spinner {
      width: 1.6rem;
      height: 1.6rem;
      border: 3px solid var(--pico-muted-border-color, #ccc);
      border-top-color: var(--pico-primary, #0172ad);
      border-radius: 50%;
      animation: mbr-genealogy-spin 0.7s linear infinite;
    }

    @keyframes mbr-genealogy-spin {
      to {
        transform: rotate(360deg);
      }
    }
  `
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-genealogy': MbrGenealogyElement
  }
}
