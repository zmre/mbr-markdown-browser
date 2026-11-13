import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { render } from 'lit'
import { formatReview } from './export-format.ts'
import { cardLocation, renderNoteCard, type NoteCardOptions } from './note-card.ts'
import { makeNote, resetNoteIds } from './test-fixtures.ts'
import type { ReviewNote } from './types.ts'

const noop = () => {}

function mount(note: ReviewNote, href: string | null = '/notes/long/path/doc/'): HTMLElement {
  const host = document.createElement('div')
  document.body.append(host)
  const options: NoteCardOptions = {
    note,
    focused: false,
    href,
    editable: true,
    confirming: false,
    onFocus: noop,
    onNavigate: noop,
    onEdit: noop,
    onDelete: noop,
    onConfirmDelete: noop,
    onCancelDelete: noop,
  }
  render(renderNoteCard(options), host)
  return host
}

beforeEach(() => resetNoteIds())
afterEach(() => {
  document.body.innerHTML = ''
})

describe('cardLocation', () => {
  it('names only the line inside a file group: the heading already names the file', () => {
    const file = 'notes/projects/a-rather-long-folder-name/meeting.md'
    expect(cardLocation(makeNote({ file, line: 15 }))).toBe(':15')
    expect(cardLocation(makeNote({ file, line: 15, endLine: 18 }))).toBe(':15-18')
    expect(cardLocation(makeNote({ file, line: null }))).toBe('whole file')
  })
})

describe('renderNoteCard', () => {
  it('shows the line, with the full location as its tooltip and accessible name', () => {
    const host = mount(makeNote({ file: 'notes/projects/meeting.md', line: 15 }))
    const loc = host.querySelector('a.note-loc')!
    expect(loc.textContent!.trim()).toBe(':15')
    expect(loc.getAttribute('title')).toBe('notes/projects/meeting.md:15')
    expect(loc.getAttribute('aria-label')).toBe('notes/projects/meeting.md:15')
  })

  it('shows the line without a link when the file has no page', () => {
    const host = mount(makeNote({ file: 'a.md', line: 3 }), null)
    const loc = host.querySelector('span.note-loc')!
    expect(loc.textContent!.trim()).toBe(':3')
    expect(loc.getAttribute('title')).toBe('a.md:3')
  })

  it("labels a suggestion's replacement text", () => {
    const host = mount(makeNote({ type: 'suggestion', suggestion: 'New wording' }))
    const block = host.querySelector('.note-suggestion-block')!
    expect(block.querySelector('.note-suggestion-label')!.textContent!.trim()).toBe('Suggested replacement')
    expect(block.querySelector('pre.note-suggestion code')!.textContent).toBe('New wording')
  })

  it('leaves the export untouched: it still names file:line', () => {
    const note = makeNote({ file: 'notes/a.md', line: 7, type: 'suggestion', suggestion: 'x' })
    expect(formatReview([note])).toContain('notes/a.md:7')
  })
})
