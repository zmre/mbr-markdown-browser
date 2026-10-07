/**
 * A rendered flashcard page, shaped like the server's output for
 *
 * ```markdown
 * Capital of France?          (5)
 * : Paris.                    (6)
 * : ___Review History___      (7)
 *   * 2026-10-06 13:45 - Again (8)
 *   * 2026-10-09 18:02 - Easy  (9)
 *
 * Two answers?                (11)
 * : one                       (12)
 * : two                       (13)
 *
 * Loose?                      (16)
 *
 * : yes                       (18)
 *
 * : **Review History**        (20)
 *
 *   * 2026-10-01 08:00 - Good  (22)
 * ```
 *
 * plus a nested list (not part of the deck) and a task with a marker below,
 * whose line references must move when a review inserts lines above them.
 */
export const DECK_HTML = `
<main id="wrapper">
<dl>
<dt data-mbr-line="5" tabindex="0">Capital of <em>France</em>?</dt>
<dd data-mbr-line="6">Paris.</dd>
<dd data-mbr-line="7"><em><strong>Review History</strong></em>
<ul>
<li data-mbr-line="8">2026-10-06 13:45 - Again</li>
<li data-mbr-line="9">2026-10-09 18:02 - Easy</li>
</ul>
</dd>
<dt data-mbr-line="11" tabindex="0">Two answers?</dt>
<dd data-mbr-line="12">one</dd>
<dd data-mbr-line="13">two <a id="anchor" href="#x">link</a></dd>
</dl>
<dl>
<dt data-mbr-line="16" tabindex="0">Loose?</dt>
<dd data-mbr-line="18"><p data-mbr-line="18">yes</p></dd>
<dd data-mbr-line="20"><p data-mbr-line="20"><strong>Review History</strong></p>
<ul>
<li data-mbr-line="22">2026-10-01 08:00 - Good</li>
</ul>
</dd>
</dl>
<blockquote><dl><dt data-mbr-line="25" tabindex="0">Nested?</dt><dd>not a card</dd></dl></blockquote>
<ul><li data-mbr-line="30"><input type="checkbox" class="mbr-task-check" id="mbr-task-30" data-mbr-task-line="30"> a task <span class="mbr-incomplete" id="mbr-marker-31">TODO</span></li></ul>
</main>
`

/** Install {@link DECK_HTML} as the page, as a `type: flashcard` note. */
export function installDeckPage(html: string = DECK_HTML): HTMLElement {
  document.body.className = 'flashcard'
  document.body.innerHTML = html
  return document.querySelector<HTMLElement>('main#wrapper')!
}

/**
 * A deck under headings, with `---` section wrappers between them, shaped
 * like the server's output for
 *
 * ```markdown
 * # Deck {#deck}
 * ## Geography          — France (Again, Easy), Spain (new)
 * ### Rivers            — Nile (Hard)
 * ---
 * ## Math               — 2+2 (Good), 3×3 (new)
 * ## Empty              — no cards
 * ```
 *
 * Card indices: 0 France, 1 Spain, 2 Nile, 3 2+2, 4 3×3. The headings carry
 * the decoration other code adds (a permalink, a review marker), which their
 * text must not include.
 */
export const SECTIONED_DECK_HTML = `
<main id="wrapper">
<h1 id="deck">Deck<a class="mbr-heading-anchor" href="#deck" aria-label="Permalink"></a></h1>
<section>
<h2 id="geo">Geography <span class="mbr-review-marker" role="button" aria-label="Note"></span></h2>
<dl>
<dt data-mbr-line="5">Capital of France?</dt>
<dd data-mbr-line="6">Paris.</dd>
<dd data-mbr-line="7"><em><strong>Review History</strong></em>
<ul>
<li data-mbr-line="8">2026-10-06 13:45 - Again</li>
<li data-mbr-line="9">2026-10-09 18:02 - Easy</li>
</ul>
</dd>
<dt data-mbr-line="11">Capital of Spain?</dt>
<dd data-mbr-line="12">Madrid.</dd>
</dl>
<h3 id="rivers">Rivers</h3>
<dl>
<dt data-mbr-line="16">Longest river?</dt>
<dd data-mbr-line="17">Nile.</dd>
<dd data-mbr-line="18"><em><strong>Review History</strong></em>
<ul><li data-mbr-line="19">2026-10-01 08:00 - Hard</li></ul>
</dd>
</dl>
</section>
<section>
<h2 id="math">Math</h2>
<dl>
<dt data-mbr-line="25">2+2?</dt>
<dd data-mbr-line="26">4</dd>
<dd data-mbr-line="27"><em><strong>Review History</strong></em>
<ul><li data-mbr-line="28">2026-10-02 08:00 - Good</li></ul>
</dd>
<dt data-mbr-line="30">3×3?</dt>
<dd data-mbr-line="31">9</dd>
</dl>
<h2 id="empty">Empty</h2>
<p>No cards here.</p>
</section>
</main>
`

/** mulberry32: a small deterministic rng, so every run makes the same picks. */
export function seededRng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
