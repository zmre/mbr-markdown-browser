import { describe, expect, it } from 'vitest'
import { taskDisplayText, taskTextSegments, wikilinkDisplay, type TextSegment } from './task-text.ts'

describe('wikilinkDisplay', () => {
  it('reads the alias, else the target, as the page draws them', () => {
    expect(wikilinkDisplay('Maya Okafor')).toBe('Maya Okafor')
    expect(wikilinkDisplay('people/maya|Maya')).toBe('Maya')
    expect(wikilinkDisplay(' Maya Okafor ')).toBe('Maya Okafor')
    expect(wikilinkDisplay('target|')).toBe('target')
    expect(wikilinkDisplay(' | ')).toBeNull()
  })
})

describe('taskTextSegments', () => {
  it('returns plain text untouched', () => {
    expect(taskTextSegments('Buy milk', null)).toEqual([{ kind: 'text', text: 'Buy milk' }])
  })

  it('turns wikilinks into their names', () => {
    expect(taskTextSegments('Call [[Maya Okafor]] about [[projects/acme|Acme]]', null)).toEqual<TextSegment[]>([
      { kind: 'text', text: 'Call ' },
      { kind: 'wikilink', text: 'Maya Okafor' },
      { kind: 'text', text: ' about ' },
      { kind: 'wikilink', text: 'Acme' },
    ])
  })

  it('leaves embeds and empty links as written', () => {
    expect(taskDisplayText('See ![[chart.png]] and [[ ]]')).toBe('See ![[chart.png]] and [[ ]]')
  })

  it('keeps the marker word at its server offsets when a wikilink comes before it', () => {
    const text = 'Ask [[people/maya|Maya]] TODO: confirm'
    const start = text.indexOf('TODO')
    const segments = taskTextSegments(text, [start, start + 4])
    expect(segments).toEqual<TextSegment[]>([
      { kind: 'text', text: 'Ask ' },
      { kind: 'wikilink', text: 'Maya' },
      { kind: 'text', text: ' ' },
      { kind: 'marker', text: 'TODO' },
      { kind: 'text', text: ': confirm' },
    ])
  })

  it('keeps UTF-16 offsets right past astral characters', () => {
    const text = '🎉 [[Party]] FIXME later'
    const start = text.indexOf('FIXME')
    const marker = taskTextSegments(text, [start, start + 5]).find((s) => s.kind === 'marker')
    expect(marker?.text).toBe('FIXME')
  })

  it('leaves a wikilink that overlaps the marker as written', () => {
    const text = 'Fix [[TODO list]] now'
    const start = text.indexOf('TODO')
    expect(taskTextSegments(text, [start, start + 4])).toEqual<TextSegment[]>([
      { kind: 'text', text: 'Fix [[' },
      { kind: 'marker', text: 'TODO' },
      { kind: 'text', text: ' list]] now' },
    ])
  })
})
