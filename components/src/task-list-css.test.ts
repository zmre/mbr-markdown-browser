import { describe, it, expect } from 'vitest'

// Read by vitest.config.ts (outside components/, and CSS imports are stubbed).
declare const __MBR_THEME_CSS__: string

/**
 * theme.css's task-list geometry. happy-dom does no layout, so the alignment
 * is pinned as the rules that carry it: a task's checkbox hangs in the
 * bullet's gutter by exactly its own width plus its gap, so the task's text
 * starts in the same column as a plain item's text at every depth. (Measured
 * in a browser: plain text and task text both at the `<li>`'s content edge,
 * the checkbox 1.65em to its left.)
 */
const css = __MBR_THEME_CSS__.replace(/\/\*[\s\S]*?\*\//g, '')

/** Top-level rules as (selectors, body), whitespace collapsed. */
function rules(): { selectors: string[]; body: string }[] {
  const out: { selectors: string[]; body: string }[] = []
  for (const match of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selectors = splitSelectors(match[1].trim().replace(/\s+/g, ' '))
    out.push({ selectors, body: match[2].replace(/\s+/g, ' ').trim() })
  }
  return out
}

/** Split a selector list on top-level commas (not those inside `:is(…)`). */
function splitSelectors(list: string): string[] {
  const parts: string[] = []
  let depth = 0
  let current = ''
  for (const ch of list) {
    if (ch === ',' && depth === 0) {
      parts.push(current.trim())
      current = ''
      continue
    }
    if (ch === '(') depth++
    else if (ch === ')') depth--
    current += ch
  }
  parts.push(current.trim())
  return parts
}

function declarationFor(selector: string, property: string): string | undefined {
  for (const rule of rules()) {
    if (!rule.selectors.includes(selector)) continue
    const match = new RegExp(`(?:^|;)\\s*${property}\\s*:\\s*([^;]+)`).exec(rule.body)
    if (match) return match[1].trim()
  }
  return undefined
}

describe('theme.css: task checkboxes stand in for the bullet', () => {
  it('drops the bullet of a task item, tight and loose', () => {
    expect(declarationFor('li:has(> .mbr-task-check)', 'list-style')).toBe('none')
    expect(declarationFor('li:has(> p > .mbr-task-check)', 'list-style')).toBe('none')
  })

  it('hangs the checkbox in the gutter by its width plus its gap', () => {
    const gap = declarationFor('.mbr-task-check', 'margin-inline-end')
    expect(gap).toBe('0.4em')
    // 1.25em is Pico's checkbox width, in the checkbox's own em.
    const hang = `calc(-1 * (1.25em + ${gap}))`
    expect(declarationFor('li > .mbr-task-check:first-child', 'margin-inline-start')).toBe(hang)
    expect(
      declarationFor('li > p:first-child > .mbr-task-check:first-child', 'margin-inline-start'),
    ).toBe(hang)
  })

  it('keeps a kanban card title task inside its card (no gutter there)', () => {
    expect(
      declarationFor(
        ':is(.kanban, .mbr-kanban-column) > :is(ul, ol) > li > .mbr-task-check:first-child',
        'margin-inline-start',
      ),
    ).toBe('0')
  })
})
