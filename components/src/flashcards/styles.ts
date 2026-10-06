/**
 * Stylesheet for `<mbr-flashcard-deck>`, injected into the document once.
 *
 * The deck renders in the **light DOM**, not a shadow root, because a card's
 * faces are clones of the page's own markdown — highlighted code, KaTeX,
 * images, tables, task chips — and only the page's stylesheets know how to
 * draw those. A shadow root would show them unstyled. The cost is that these
 * rules are global, hence the `.mbr-fc-` prefix on every class; the upside is
 * that `.mbr/user.css` can restyle the deck like anything else.
 *
 * Colours come from Pico's custom properties, so light/dark themes and every
 * `--theme` variant follow without a rule of their own.
 */
export const DECK_STYLE_ID = 'mbr-flashcards-styles'

export const DECK_CSS = `
.mbr-fc-overlay {
  position: fixed;
  inset: 0;
  z-index: 1100;
  display: flex;
  flex-direction: column;
  background: var(--pico-background-color, #fff);
  color: var(--pico-color, #333);
  animation: mbr-fc-fade-in 0.18s ease-out;
}
@keyframes mbr-fc-fade-in { from { opacity: 0; } to { opacity: 1; } }

.mbr-fc-bar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.5rem 0.75rem;
  padding: 0.6rem clamp(0.75rem, 3vw, 1.5rem);
  border-bottom: 1px solid var(--pico-muted-border-color, #e5e7eb);
}
.mbr-fc-title {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
  white-space: nowrap;
}
.mbr-fc-counter {
  font-size: 0.85rem;
  color: var(--pico-muted-color, #6b7280);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.mbr-fc-spacer { flex: 1; }
.mbr-fc-overlay button,
.mbr-fc-overlay select {
  width: auto;
  margin: 0;
  font-size: 0.85rem;
  line-height: 1.2;
}
.mbr-fc-bar button,
.mbr-fc-bar select {
  padding: 0.35rem 0.7rem;
  height: auto;
}
.mbr-fc-bar select { padding-inline-end: 2rem; }
.mbr-fc-swap[aria-pressed="true"] {
  background: var(--pico-primary-background, #0172ad);
  border-color: var(--pico-primary-background, #0172ad);
  color: var(--pico-primary-inverse, #fff);
}
.mbr-fc-close {
  border: none;
  background: transparent;
  /* inherit, not --pico-color: Pico redefines that on every button (to the
   * primary-inverse white), which made this glyph invisible in light mode. */
  color: inherit;
  font-size: 1.4rem !important;
  line-height: 1;
  padding: 0.15rem 0.5rem !important;
}
.mbr-fc-close:hover { background: var(--pico-secondary-background, rgba(0, 0, 0, 0.06)); }

.mbr-fc-banner {
  margin: 0.6rem auto 0;
  padding: 0.5rem 0.9rem;
  max-width: min(900px, calc(100% - 1.5rem));
  border-radius: var(--pico-border-radius, 0.25rem);
  border: 1px solid var(--pico-del-color, #c62828);
  color: var(--pico-del-color, #c62828);
  font-size: 0.9rem;
  display: flex;
  gap: 0.75rem;
  align-items: center;
}
.mbr-fc-banner span { flex: 1; }

.mbr-fc-stage {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: stretch;
  justify-content: center;
  padding: clamp(0.75rem, 3vw, 2rem);
}

/* One slot per card (re-created on every card change), so each new card
 * animates in on its own -- and never inherits the previous card's flip. */
.mbr-fc-slot {
  flex: 1;
  display: flex;
  justify-content: center;
  perspective: 1600px;
  animation: mbr-fc-enter 0.28s ease-out;
}

.mbr-fc-card {
  position: relative;
  flex: 1;
  max-width: 960px;
  min-height: 12rem;
  cursor: pointer;
  transform-style: preserve-3d;
  transition: transform 0.55s cubic-bezier(0.2, 0.7, 0.2, 1);
  outline: none;
  border-radius: 1rem;
}
@keyframes mbr-fc-enter {
  from { opacity: 0; transform: translateY(10px) scale(0.985); }
  to { opacity: 1; transform: none; }
}
.mbr-fc-card.is-flipped { transform: rotateY(180deg); }
.mbr-fc-card:focus-visible .mbr-fc-face {
  /* Pico's focus colour is translucent: a ring, not a second border. */
  box-shadow: 0 0 0 3px var(--pico-primary-focus, rgba(1, 114, 173, 0.35));
}

.mbr-fc-face {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
  padding: clamp(2rem, 5vw, 3.25rem) clamp(1rem, 4vw, 3rem) clamp(1.25rem, 4vw, 2.5rem);
  border-radius: 1rem;
  border: 1px solid var(--pico-muted-border-color, #e5e7eb);
  background: var(--pico-card-background-color, var(--pico-background-color, #fff));
  box-shadow: var(--pico-card-box-shadow, 0 0.5rem 1.5rem rgba(0, 0, 0, 0.08)), 0 1.5rem 3rem -1.5rem rgba(0, 0, 0, 0.25);
  backface-visibility: hidden;
  -webkit-backface-visibility: hidden;
}
.mbr-fc-back { transform: rotateY(180deg); }
.mbr-fc-side {
  position: absolute;
  top: 0.85rem;
  left: 1.1rem;
  font-size: 0.7rem;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--pico-muted-color, #6b7280);
}
.mbr-fc-back .mbr-fc-side { color: var(--pico-primary, #0172ad); }

/* margin: auto centres the content while it fits and lets it scroll from the
 * top once it does not -- align-items: center would clip its top edge. */
.mbr-fc-content {
  margin: auto;
  max-width: 100%;
  text-align: center;
  line-height: 1.35;
  overflow-wrap: anywhere;
}
.mbr-fc-content > :first-child { margin-top: 0; }
.mbr-fc-content > :last-child { margin-bottom: 0; }
.mbr-fc-content :is(p, ul, ol, pre, table, blockquote, figure) { margin-block: 0 0.6em; }
.mbr-fc-content :is(ul, ol, pre, table, blockquote, dl) {
  text-align: start;
  width: fit-content;
  max-width: 100%;
  margin-inline: auto;
}
.mbr-fc-content :is(pre, code, kbd) { font-size: 0.8em; }
.mbr-fc-content :is(img, video, svg) {
  max-width: 100%;
  max-height: 55vh;
  height: auto;
  object-fit: contain;
}
.mbr-fc-answer + .mbr-fc-answer {
  margin-top: 0.6em;
  padding-top: 0.6em;
  border-top: 1px dashed var(--pico-muted-border-color, #e5e7eb);
}

.mbr-fc-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  padding: 0 clamp(0.75rem, 3vw, 1.5rem) 0.5rem;
  min-height: 3.25rem;
}
.mbr-fc-controls button { padding: 0.55rem 1rem; }
.mbr-fc-primary { min-width: 10rem; }

.mbr-fc-ratings {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 0.5rem;
  width: min(100%, 640px);
}
.mbr-fc-ratings button {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.15rem;
  padding: 0.5rem 0.25rem;
  border-width: 1px;
  border-style: solid;
  background: transparent;
  color: var(--mbr-fc-rating, var(--pico-color));
  border-color: var(--mbr-fc-rating, var(--pico-muted-border-color));
  font-weight: 600;
}
.mbr-fc-ratings button:hover:not(:disabled),
.mbr-fc-ratings button.is-pending {
  background: var(--mbr-fc-rating, var(--pico-primary));
  color: var(--pico-background-color, #fff);
}
.mbr-fc-ratings button small {
  font-weight: 400;
  font-size: 0.75rem;
  opacity: 0.85;
  font-variant-numeric: tabular-nums;
}
.mbr-fc-ratings kbd {
  font-size: 0.7rem;
  padding: 0 0.3rem;
  margin-inline-end: 0.3rem;
  background: transparent;
  color: inherit;
  border: 1px solid currentColor;
  opacity: 0.7;
}
.mbr-fc-rate-again { --mbr-fc-rating: var(--pico-del-color, #c62828); }
.mbr-fc-rate-hard { --mbr-fc-rating: #b7791f; }
.mbr-fc-rate-good { --mbr-fc-rating: var(--pico-ins-color, #2e7d32); }
.mbr-fc-rate-easy { --mbr-fc-rating: var(--pico-primary, #0172ad); }
.mbr-fc-ratings.is-nudged { animation: mbr-fc-nudge 0.35s ease-in-out; }
@keyframes mbr-fc-nudge {
  0%, 100% { transform: none; }
  30% { transform: translateY(-4px); }
  60% { transform: translateY(2px); }
}

.mbr-fc-history-toggle {
  background: transparent;
  border: none;
  color: var(--pico-muted-color, #6b7280);
  text-decoration: underline;
  text-underline-offset: 3px;
}
.mbr-fc-history-panel {
  margin: 0 auto 0.5rem;
  width: min(100% - 1.5rem, 640px);
  max-height: 30vh;
  overflow: auto;
  font-size: 0.85rem;
  border: 1px solid var(--pico-muted-border-color, #e5e7eb);
  border-radius: var(--pico-border-radius, 0.25rem);
  padding: 0.5rem 0.75rem;
}
.mbr-fc-history-panel table { margin: 0; font-size: inherit; }
.mbr-fc-history-panel :is(th, td) { padding: 0.2rem 0.5rem; }
.mbr-fc-history-panel p { margin: 0 0 0.4rem; color: var(--pico-muted-color, #6b7280); }

.mbr-fc-hints {
  margin: 0;
  padding: 0 1rem 0.6rem;
  text-align: center;
  font-size: 0.75rem;
  color: var(--pico-muted-color, #6b7280);
}
.mbr-fc-hints kbd { font-size: 0.7rem; padding: 0.05rem 0.3rem; }

.mbr-fc-screen {
  margin: auto;
  text-align: center;
  max-width: 32rem;
}
.mbr-fc-screen h3 { margin-bottom: 0.5rem; }
.mbr-fc-screen p { color: var(--pico-muted-color, #6b7280); }
.mbr-fc-screen .mbr-fc-actions {
  display: flex;
  gap: 0.6rem;
  justify-content: center;
  flex-wrap: wrap;
}

.mbr-fc-sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}

@media (max-width: 576px) {
  .mbr-fc-title { display: none; }
  .mbr-fc-hints { display: none; }
  .mbr-fc-swap-label { display: none; }
  .mbr-fc-ratings { gap: 0.3rem; }
}

/* No 3D turn: cross-fade the faces instead. */
@media (prefers-reduced-motion: reduce) {
  .mbr-fc-overlay, .mbr-fc-slot, .mbr-fc-ratings.is-nudged { animation: none; }
  .mbr-fc-card, .mbr-fc-card.is-flipped { transform: none; transition: none; }
  .mbr-fc-face { transition: opacity 0.15s linear; }
  .mbr-fc-back { transform: none; opacity: 0; }
  .mbr-fc-card.is-flipped .mbr-fc-back { opacity: 1; }
  .mbr-fc-card.is-flipped .mbr-fc-front { opacity: 0; }
}
`

/** Add the deck's stylesheet to `doc` once. */
export function ensureDeckStyles(doc: Document = document): void {
  if (doc.getElementById(DECK_STYLE_ID)) return
  const style = doc.createElement('style')
  style.id = DECK_STYLE_ID
  style.textContent = DECK_CSS
  doc.head.append(style)
}
