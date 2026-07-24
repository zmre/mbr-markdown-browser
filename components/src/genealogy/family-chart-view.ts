/**
 * family-chart (donatso, v0.9) view — the default genealogy chart.
 *
 * Verified against the shipped `dist/types`:
 *  - data format is v2: `{id, data: {...}, rels: {parents[], spouses[], children[]}}`;
 *  - `createChart(cont, data)` defaults `main_id` to the first datum, so the
 *    focus is set explicitly via `updateMainId()`;
 *  - SVG cards via `chart.setCardSvg()` with `setCardDisplay` (one function per
 *    text line, each receiving the `Datum`), `setMiniTree` + `setLinkBreak`
 *    (the hide/expand toggles for non-blood/hidden branches), and
 *    `setOnCardClick`;
 *  - `CardImage` renders `data.avatar` with built-in silhouette fallbacks;
 *  - the library binds no document/window listeners for this feature set (only
 *    the unused edit-form/search features do), so teardown = remove the DOM.
 *
 * The library CSS is imported `?inline` and injected into the mount's root
 * node (shadow-root safe), followed by overrides mapping the hardcoded dark
 * palette onto Pico variables + the shared `--mbr-gen-*` gender props.
 */
import { createChart, handlers } from 'family-chart'
import type { Data, Datum, TreeDatum } from 'family-chart'
import familyChartCss from 'family-chart/styles/family-chart.css?inline'
import { formatLifespan } from '../graph/relationship-graph.js'
import { DRAG_THRESHOLD_PX } from '../graph/viewport.js'
import { toFamilyChartData } from './family-chart-data.js'
import { computeInitialViewBox } from './timeline-layout.js'
import {
  injectStylesOnce,
  type GenealogyChart,
  type GenealogyChartInstance,
  type GenealogyContext,
} from './chart-registry.js'

/** Generations shown above/below the focused person. */
const ANCESTRY_DEPTH = 2
const PROGENY_DEPTH = 2
const TRANSITION_MS = 600

/**
 * Card geometry: the library's stock 220×70 card widened ~15% → 253×70.
 * Portrait column (60×60 at 5,5) and the text start (`text_x: 75` = img right
 * edge + 10px gap) keep the stock proportions.
 *
 * `text_y: 18` centers the two-line text block: tspan baselines sit at a
 * FIXED 14px pitch (`dy="14"` hardcoded in the library's CardText template),
 * so baselines land at text_y+14 / text_y+28 = 32 / 46; with a 13px line 1
 * (cap ascent ≈ 9.6px) and 10px line 2 (descent ≈ 2.2px) the block spans
 * ≈ 22.4→48.2, centered on the 70px card.
 */
const CARD_DIM = { w: 253, h: 70, text_x: 75, text_y: 18, img_w: 60, img_h: 60, img_x: 5, img_y: 5 }
/** Horizontal gap between neighbouring cards (node separation = w + gap). */
const CARD_GAP = 30
/** Text x for cards WITHOUT a portrait (image column removed, text takes over). */
const NO_IMAGE_TEXT_X = 12
/**
 * Card text font sizes. The 14px baseline pitch is fixed (see above), so the
 * sizes must satisfy: line-1 descent (13 × ~0.22 ≈ 2.9px) + line-2 ascent
 * (10 × ~0.74 ≈ 7.4px) ≈ 10.3px < 14px — no overlap, ~3.7px clearance.
 * (Previously both lines inherited the browser-default 16px, whose descent +
 * ascent exceeds 14px — the source of the overlap.)
 */
const LINE1_FONT_PX = 13
const LINE2_FONT_PX = 10

/**
 * Smart-initial-zoom readability thresholds (see the timeline's constants for
 * the shared rationale). When the library's fit-all scale would render the
 * 70px cards below `MIN_READABLE_CARD_PX` on screen, the chart starts partly
 * zoomed in at `TARGET_READABLE_CARD_PX` instead, centered on the main person.
 *
 * Values are keyed to on-screen TITLE text so both charts feel the same: the
 * 70px card carries a 13px title, so 44px ⇒ ≈8.2px text (legibility floor,
 * matching the timeline's ≈8.5px trigger) and 60px ⇒ ≈11.1px text (matching
 * the timeline's ≈11.3px start). Note 60/70 < 1, so the synthetic-fit trick
 * below is never affected by the library's k ≤ 1 fit cap.
 */
const MIN_READABLE_CARD_PX = 44
const TARGET_READABLE_CARD_PX = 60

function mountFamilyChart(container: HTMLElement, ctx: GenealogyContext): GenealogyChartInstance {
  injectStylesOnce(container.getRootNode(), 'mbr-genealogy-f3', `${familyChartCss}\n${F3_THEME_CSS}`)

  const cont = document.createElement('div')
  cont.className = 'f3 f3-cont mbr-f3'
  container.appendChild(cont)

  const { data, mainId } = toFamilyChartData(ctx.graph)
  // The pure converter keeps raw frontmatter image paths; resolve them for the
  // current page (server absolute vs static relative) here at the view edge.
  const resolved = data.map((d) =>
    d.data.avatar ? { ...d, data: { ...d.data, avatar: ctx.resolveUrl(d.data.avatar) } } : d
  )

  // Our data legitimately omits `gender` for unknown values; family-chart's
  // Datum types it as required but the runtime renders a genderless card.
  const chart = createChart(cont, resolved as unknown as Data)
  chart
    .setTransitionTime(TRANSITION_MS)
    .setOrientationVertical()
    .setAncestryDepth(ANCESTRY_DEPTH)
    .setProgenyDepth(PROGENY_DEPTH)
    .setShowSiblingsOfMain(true)
    .setSingleParentEmptyCard(false)
    .setCardXSpacing(CARD_DIM.w + CARD_GAP)
  chart.updateMainId(mainId)

  // Belt-and-braces click-vs-drag guard: family-chart's d3-zoom suppresses
  // clicks after drags natively, but we also compare against the pointerdown
  // position with the shared threshold.
  const listeners = new AbortController()
  let downAt: { x: number; y: number } | null = null
  cont.addEventListener('pointerdown', (e) => (downAt = { x: e.clientX, y: e.clientY }), {
    capture: true,
    signal: listeners.signal,
  })

  const card = chart.setCardSvg()
  card
    .setCardDisplay([
      (d: Datum) => String(d.data['label'] ?? ''),
      (d: Datum) =>
        formatLifespan(
          typeof d.data['birthday'] === 'string' ? d.data['birthday'] : undefined,
          typeof d.data['death'] === 'string' ? d.data['death'] : undefined
        ),
    ])
    .setCardDim({ ...CARD_DIM })
    .setMiniTree(true)
    .setLinkBreak(true)
    // For cards WITHOUT a portrait: drop the image area entirely (no
    // gender-silhouette placeholder) and slide the text left into the freed
    // space. `onCardUpdate` runs per card after assembly with `this` bound to
    // the card's DOM node; the library's `card_text_clip` spans the full card
    // width, so the shifted text is not clipped.
    .setOnCardUpdate(function (this: unknown, d: TreeDatum) {
      if (!(this instanceof Element)) return
      const avatar = d.data.data['avatar']
      if (typeof avatar === 'string' && avatar.length > 0) return
      this.querySelector('.card_image')?.remove()
      this.querySelector('.card-text > g')?.setAttribute(
        'transform',
        `translate(${NO_IMAGE_TEXT_X}, ${CARD_DIM.text_y})`
      )
    })
    .setOnCardClick((e: MouseEvent, d: TreeDatum) => {
      if (downAt && Math.hypot(e.clientX - downAt.x, e.clientY - downAt.y) > DRAG_THRESHOLD_PX) {
        return
      }
      ctx.navigate(String(d.data.id))
    })

  /**
   * Smart view: keep the library's fit-all when cards stay readable; for very
   * wide/shallow trees position the view at a readable zoom centered on the
   * main person (clamped to the tree bounds) instead.
   *
   * Mechanism (public API only): the library's zoom lives in `positionTree`,
   * which is not exported — but the exported `treeFit` drives it through
   * `calculateTreeFit(svg_dim, tree_dim) → {k, x, y}` with
   * `k = min(svgW/w, svgH/h)` (≤1), `x = x_off + (svgW − w·k)/k/2`, `y = …`.
   * Feeding it a SYNTHETIC `tree_dim` of `{width: svgW/k_t, height: svgH/k_t,
   * x_off: t_x, y_off: t_y}` therefore lands EXACTLY on the desired transform
   * `{k: k_t, x: t_x, y: t_y}` (both min-ratios equal k_t, so the centering
   * terms vanish). Scheduling it right after `updateTree({initial: true})`
   * also cleanly REPLACES the library's own pending fit, because both go
   * through the same default-name d3 transition on the same element.
   * (The exported `cardToMiddle` is not used: its y term omits `· k` and
   * mis-centers vertically at any scale ≠ 1.)
   *
   * The library's `setupZoom` applies no `scaleExtent` (d3's default is
   * [0, ∞]), so manual zoom-out back to — and past — full fit stays possible.
   *
   * Returns false when the view could not be determined (no tree/dims/main
   * card yet); callers then leave the library's default fit untouched.
   */
  const applySmartView = (transitionTime: number): boolean => {
    try {
      const svg = chart.svg
      const rect = svg.getBoundingClientRect()
      if (!(rect.width > 0) || !(rect.height > 0)) return false
      const dim = chart.store.getTree()?.dim
      if (!dim || !(dim.width > 0) || !(dim.height > 0)) return false
      const svgDim = { width: rect.width, height: rect.height }

      const fit = handlers.calculateTreeFit(svgDim, dim)
      if (CARD_DIM.h * fit.k >= MIN_READABLE_CARD_PX) {
        // Fit-all is readable. On recenter, animate back to it; at mount the
        // library's own initial fit is already in flight — leave it alone.
        if (transitionTime > 0) {
          handlers.treeFit({ svg, svg_dim: svgDim, tree_dim: dim, transition_time: transitionTime })
        }
        return true
      }

      // The main person's card, in content coordinates (tree coordinates are
      // offset by −x_off/−y_off; fitting translates by +x_off/+y_off).
      const main = chart.store.getTreeMainDatum()
      if (!main || !Number.isFinite(main.x) || !Number.isFinite(main.y)) return false
      const view = computeInitialViewBox({
        contentWidth: dim.width,
        contentHeight: dim.height,
        canvasWidth: rect.width,
        canvasHeight: rect.height,
        cardH: CARD_DIM.h,
        focusX: main.x + dim.x_off,
        focusY: main.y + dim.y_off,
        minReadablePx: MIN_READABLE_CARD_PX,
        targetPx: TARGET_READABLE_CARD_PX,
      })
      // view.{x,y,w,h} (content coords) → the synthetic dims described above:
      // k_t = svgW/view.w and the translate that puts view's origin at 0.
      handlers.treeFit({
        svg,
        svg_dim: svgDim,
        tree_dim: {
          width: view.w,
          height: view.h,
          x_off: dim.x_off - view.x,
          y_off: dim.y_off - view.y,
        },
        transition_time: transitionTime,
      })
      return true
    } catch {
      // e.g. the store throws before the first tree render; use library fit.
      return false
    }
  }

  chart.updateTree({ initial: true })
  applySmartView(0)

  // Recenter control: return to the same smart view (fit-all when readable,
  // else the readable zoom centered on the main person).
  const controls = document.createElement('div')
  controls.className = 'rel-graph-controls'
  const recenter = document.createElement('button')
  recenter.type = 'button'
  recenter.textContent = '⤢'
  recenter.setAttribute('aria-label', 'Recenter')
  recenter.title = 'Recenter'
  recenter.addEventListener(
    'click',
    () => {
      if (!applySmartView(400)) chart.updateTree({ initial: true })
    },
    { signal: listeners.signal }
  )
  controls.appendChild(recenter)
  cont.appendChild(controls)

  return {
    destroy() {
      listeners.abort()
      // family-chart attaches its listeners to elements inside `cont` (no
      // document/window listeners for the features used here), so removing the
      // subtree tears everything down.
      cont.remove()
    },
  }
}

export const familyChartType: GenealogyChart = {
  id: 'family-chart',
  label: 'Family chart',
  mount: mountFamilyChart,
}

/**
 * Map family-chart's hardcoded dark palette onto the page theme: Pico
 * variables for surface/text plus the shared `--mbr-gen-*` gender props (the
 * base sheet flips those for dark mode). Loaded after the library CSS so these
 * override it.
 */
const F3_THEME_CSS = `
.f3.f3-cont.mbr-f3 {
  width: 100%;
  height: 100%;
  max-height: none;
  position: relative;
  border-radius: 4px;
  background-color: var(--pico-card-background-color, #fff);
  color: var(--pico-color, #222);
  font-family: var(--pico-font-family, sans-serif);
}

.f3.mbr-f3 {
  --male-color: var(--mbr-gen-male-fill, #d7e3f8);
  --female-color: var(--mbr-gen-female-fill, #f8d7e3);
  --genderless-color: var(--pico-muted-border-color, lightgray);
}

/* Cursor affordances. The pan/zoom surface is the #f3Canvas div (d3-zoom is
   bound there and sets no cursor itself): grab, and grabbing while a drag is
   in progress (:active). The cursor property resolves per element, so the card
   rule below still wins whenever the pointer is over a card — even mid-drag. */
.f3.mbr-f3 #f3Canvas {
  cursor: grab;
}

.f3.mbr-f3 #f3Canvas:active {
  cursor: grabbing;
}

/* Person cards are clickable (the library binds click on g.card but never
   styles it). Covers image and imageless variants alike; the mini-tree toggle
   (.card_family_tree) already carries an inline cursor: pointer. */
.f3.mbr-f3 svg.main_svg g.card {
  cursor: pointer;
}

/* Card text: the library hardcodes a 14px tspan baseline pitch but sets no
   font-size (cards inherit the 16px browser default, which overlaps at that
   pitch). Explicit sizes fit the pitch; line 1 stays dominant. */
.f3.mbr-f3 .card-text text {
  font-size: ${LINE1_FONT_PX}px;
  font-weight: 600;
}

.f3.mbr-f3 .card-text text tspan:nth-of-type(2) {
  font-size: ${LINE2_FONT_PX}px;
  font-weight: 400;
  opacity: 0.85;
}

/* Belt-and-braces with the onCardUpdate removal: never show the library's
   gender-silhouette placeholder on cards without a portrait. */
.f3.mbr-f3 .card_image .genderless-icon {
  display: none;
}

/* The library sets stroke="#fff" as a presentation attribute on tree links
   (invisible on light surfaces); CSS rules outrank presentation attributes. */
.f3.mbr-f3 path.link {
  stroke: var(--pico-muted-color, #8a8a8a);
}

/* Keep the link-break / mini-tree toggle icons legible on light surfaces. */
.f3.mbr-f3 .card_break_link,
.f3.mbr-f3 .card_family_tree {
  color: var(--pico-muted-color, #777);
}
`
