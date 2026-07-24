/**
 * `<mbr-genealogy>` — lightweight trigger for the person-page genealogy charts.
 *
 * Lives in the main bundle and stays tiny: it guards on `type: person`
 * frontmatter, builds the relationship graph from site.json, renders a
 * fixed-height placeholder (no layout shift) and, once the placeholder nears
 * the viewport (IntersectionObserver, 400px margin), dynamically imports the
 * heavy `mbr-genealogy.min.js` chunk (family-chart + timeline tree) and hands
 * it the graph via `mountGenealogy()`. Pages without a person focus or without
 * any resolved relationship edges render nothing at all.
 */
import { LitElement, html, css, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { waitForDom, getMbrAssetBase } from './dynamic-loader.ts'
import { subscribeSiteNav, getCanonicalPath, resolveUrl } from './shared.ts'
import {
  DEFAULT_DEPTH,
  DEFAULT_MAX_NODES,
  buildRegistry,
  buildRelationshipGraph,
  notesByPathFromSite,
  type RelationshipGraph,
  type RelationTypeConfig,
  type Registry,
  type SiteNote,
} from './graph/relationship-graph.js'
import type { GenealogyContext, GenealogyController } from './genealogy/index.js'

/** Shape of the lazily-loaded genealogy chunk (type-only; erased at build). */
type GenealogyModule = {
  mountGenealogy(container: HTMLElement, ctx: GenealogyContext): GenealogyController
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
  /** How many relationship hops to expand outward from the focused person. */
  @property({ type: Number })
  depth = DEFAULT_DEPTH

  /** Safety cap on graph size for very large repositories. */
  @property({ type: Number, attribute: 'max-nodes' })
  maxNodes = DEFAULT_MAX_NODES

  @state()
  private _graph: RelationshipGraph | null = null

  @state()
  private _mounted = false

  @state()
  private _failed = false

  private _siteData: { markdown_files?: SiteNote[]; relationship_types?: RelationTypeConfig[] } | null =
    null
  private _notesByPath: Map<string, SiteNote> = new Map()
  private _registry: Registry | null = null
  private _unsubscribeSiteNav?: () => void
  private _observer: IntersectionObserver | null = null
  private _loadArmed = false
  private _loadPromise: Promise<void> | null = null
  private _module: GenealogyModule | null = null
  private _controller: GenealogyController | null = null

  override connectedCallback() {
    super.connectedCallback()
    void waitForDom().then(() => {
      // Only person pages get a genealogy chart.
      if (window.frontmatter?.['type'] !== 'person') return
      this._unsubscribeSiteNav = subscribeSiteNav((state) => {
        if (state.data && state.data !== this._siteData) {
          this._siteData = state.data
          this._rebuildGraph()
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
      this._rebuildGraph()
    }
    if (this._hasChart() && !this._loadArmed) {
      this._armLoad()
    }
  }

  private _hasChart(): boolean {
    return !this._failed && this._graph !== null && this._graph.edges.length > 0
  }

  private _rebuildGraph(): void {
    const data = this._siteData
    if (!data) return
    this._notesByPath = notesByPathFromSite(data)
    const types = Array.isArray(data.relationship_types) ? data.relationship_types : []
    this._registry = buildRegistry(types)
    this._graph = buildRelationshipGraph(
      getCanonicalPath(),
      this._notesByPath,
      this._registry,
      this.depth,
      this.maxNodes
    )
    // A depth/max-nodes change after mount: remount the chart with the new graph.
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
        console.warn('[mbr-genealogy] Failed to load the genealogy chart chunk:', err)
        this._failed = true
      }
    })()
    return this._loadPromise
  }

  private async _mountChart(): Promise<void> {
    const graph = this._graph
    if (!this._module || !graph || graph.edges.length === 0 || this._controller) return
    // Make sure the mount container from the current template is in the DOM.
    await this.updateComplete
    const container = this.shadowRoot?.querySelector<HTMLElement>('.gen-mount')
    if (!container || !this._registry) return
    this._controller = this._module.mountGenealogy(container, {
      graph,
      notesByPath: this._notesByPath,
      registry: this._registry,
      focusPath: graph.focus,
      resolveUrl,
      navigate: (path: string) => window.location.assign(resolveUrl(path)),
    })
    this._mounted = true
  }

  override render() {
    if (!this._hasChart()) return nothing
    return html`
      <figure class="gen-figure" role="group" aria-label="Family charts">
        <figcaption>Family tree</figcaption>
        <div class="gen-canvas">
          <div class="gen-mount"></div>
          ${this._mounted
            ? nothing
            : html`
                <div class="gen-loading" role="status" aria-label="Loading family chart">
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
       layout shift when it mounts. */
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
