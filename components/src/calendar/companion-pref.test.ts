import { afterEach, describe, expect, it } from 'vitest'
import { COMPANION_HIDDEN_CLASS, COMPANION_HIDDEN_KEY, isCompanionHidden, setCompanionHidden } from './companion-pref.ts'

declare const __MBR_HEAD_TEMPLATE__: string

afterEach(() => {
  document.documentElement.classList.remove(COMPANION_HIDDEN_CLASS)
  localStorage.removeItem(COMPANION_HIDDEN_KEY)
})

describe('the companion dismissal preference', () => {
  it('sets the class and remembers it, and clears both', () => {
    setCompanionHidden(true)
    expect(isCompanionHidden()).toBe(true)
    expect(localStorage.getItem(COMPANION_HIDDEN_KEY)).toBe('1')
    setCompanionHidden(false)
    expect(isCompanionHidden()).toBe(false)
    expect(localStorage.getItem(COMPANION_HIDDEN_KEY)).toBeNull()
  })

  it('is read before paint by _head.html, under the same key and class', () => {
    const script = /<script>[^<]*mbr_calendar_companion_hidden[^<]*<\/script>/.exec(__MBR_HEAD_TEMPLATE__)?.[0]
    expect(script).toBeDefined()
    expect(script).toContain(`localStorage.getItem("${COMPANION_HIDDEN_KEY}") === "1"`)
    expect(script).toContain(`classList.add("${COMPANION_HIDDEN_CLASS}")`)

    // And it works: run the script's body against this document.
    localStorage.setItem(COMPANION_HIDDEN_KEY, '1')
    new Function(script!.replace(/^<script>|<\/script>$/g, ''))()
    expect(isCompanionHidden()).toBe(true)
  })
})
