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
