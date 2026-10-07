import { describe, expect, it } from 'vitest'
import { DECK_CSS } from './styles.js'
import { HISTORY_CLASS } from './dom.js'
import { LAST_CLASS_PREFIX, PIE_BUCKETS, PIE_CLASS } from './progress.js'

/** `templates/theme.css`, injected by vitest.config.ts (see the note there). */
declare const __MBR_THEME_CSS__: string

/** The body of the first rule whose selector is exactly `selector`. */
function ruleBody(selector: string): string | undefined {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\>]/g, '\\$&')
  return new RegExp(`(?:^|})\\s*${escaped}\\s*\\{([^}]*)\\}`, 'm').exec(__MBR_THEME_CSS__)?.[1]
}

/**
 * happy-dom does no layout, so the overlap itself can only be seen in a
 * browser (it was: "set a page's `<body>` classes?" on the flashcards docs
 * page). This pins the rule that fixes it.
 */
describe('definition-list questions in theme.css', () => {
  it('hang their indent on the dt', () => {
    expect(ruleBody('main dl > dt')).toMatch(/text-indent:\s*calc\(\s*-1/)
  })

  it('reset the inherited negative indent on inline-blocks inside (code, kbd)', () => {
    expect(ruleBody('main dl > dt *')).toMatch(/text-indent:\s*0/)
  })
})

describe('flashcard progress and rating colours in theme.css', () => {
  it('define the four rating colours once, with dark-mode Again', () => {
    for (const rating of ['again', 'hard', 'good', 'easy', 'unreviewed']) {
      expect(__MBR_THEME_CSS__).toMatch(new RegExp(`--mbr-fc-${rating}:`))
    }
    expect(__MBR_THEME_CSS__).toMatch(/\[data-theme="dark"\]\s*\{\s*--mbr-fc-again:\s*#ef5350/)
  })

  it('border a reviewed question in its rating colour, inside main only', () => {
    for (const rating of ['again', 'hard', 'good', 'easy']) {
      expect(ruleBody(`main dl > dt.${LAST_CLASS_PREFIX}${rating}`)).toMatch(
        new RegExp(`--mbr-fc-last:\\s*var\\(--mbr-fc-${rating}\\)`)
      )
    }
    expect(__MBR_THEME_CSS__).toMatch(/border-left:\s*2px solid var\(--mbr-fc-last\)/)
  })

  it('size the pie and colour every slice', () => {
    expect(ruleBody(`main .${PIE_CLASS}`)).toMatch(/width:\s*0\.8em/)
    for (const bucket of PIE_BUCKETS) expect(ruleBody(`main .${PIE_CLASS}-${bucket}`)).toMatch(/fill:/)
  })

  it('leave the deck sheet referencing, not redefining, the colours', () => {
    expect(DECK_CSS).not.toMatch(/--mbr-fc-again\s*:/)
    expect(DECK_CSS).toMatch(/var\(--mbr-fc-hard/)
    // The deck sheet stays global after a close; it must never reach the
    // page's own history, borders or pies.
    expect(DECK_CSS).not.toMatch(new RegExp(`\\.${HISTORY_CLASS}[\\s{:,.]`))
    expect(DECK_CSS).not.toContain(`.${LAST_CLASS_PREFIX}`)
    expect(DECK_CSS).not.toMatch(new RegExp(`\\.${PIE_CLASS}[\\s{:,.-]`))
  })
})
