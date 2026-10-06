import { describe, expect, it } from 'vitest'

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
