import { describe, expect, it } from 'vitest'
import { agendaEntries, formatMinutes, leadingText, parseAgendaText } from './agenda-parse.ts'
import { AGENDA_PREFILTER } from './agenda-scope.ts'

const hm = (h: number, m = 0): number => h * 60 + m

/** `[line, start, end | undefined, title]`; `null` = not an entry. */
const GRAMMAR: ReadonlyArray<readonly [string, readonly [number, number | undefined, string] | null]> = [
  // The owner's daily-note `## Calendar` lines.
  ['09:00 - 09:15 Standup', [hm(9), hm(9, 15), 'Standup']],
  ['10:30 - 11:00 Bob / Patrick 1:1', [hm(10, 30), hm(11), 'Bob / Patrick 1:1']],
  [
    '14:00 - 15:00 Things we did / Rev team demos / experiment results / kudos',
    [hm(14), hm(15), 'Things we did / Rev team demos / experiment results / kudos'],
  ],
  // Obsidian Day Planner (the checkbox is outside the text).
  ['10:00 - 10:30 Wake up', [hm(10), hm(10, 30), 'Wake up']],
  ['09.00 - 09.30 Plan the day', [hm(9), hm(9, 30), 'Plan the day']],
  ['13:00 Lunch', [hm(13), undefined, 'Lunch']],
  // NotePlan time blocks.
  ['at 3pm Call the bank', [hm(15), undefined, 'Call the bank']],
  ['from 9am to 10:30am Workshop', [hm(9), hm(10, 30), 'Workshop']],
  ['5:00pm-6pm Gym', [hm(17), hm(18), 'Gym']],
  ['noon Lunch with Sam', [hm(12), undefined, 'Lunch with Sam']],
  ['midnight Deploy window', [0, undefined, 'Deploy window']],
  ['11pm - midnight Wind down', [hm(23), hm(24), 'Wind down']],
  // am/pm spellings and spacing, case-insensitive.
  ['9 AM Standup', [hm(9), undefined, 'Standup']],
  ['9:30 a.m. Coffee', [hm(9, 30), undefined, 'Coffee']],
  ['12:15 PM Lunch', [hm(12, 15), undefined, 'Lunch']],
  ['12am Night shift', [0, undefined, 'Night shift']],
  ['AT 7:45 Train', [hm(7, 45), undefined, 'Train']],
  // Separators: en and em dashes, "to", a colon or dash before the title.
  ['9:00–10:00 Review', [hm(9), hm(10), 'Review']],
  ['9:00 — 10:00 Review', [hm(9), hm(10), 'Review']],
  ['9:00 to 10:00 Review', [hm(9), hm(10), 'Review']],
  ['14:00: Lunch', [hm(14), undefined, 'Lunch']],
  ['14:00 - Lunch', [hm(14), undefined, 'Lunch']],
  ['  08:30 Leading spaces', [hm(8, 30), undefined, 'Leading spaces']],
  // An end without am/pm inherits the start's half when that is later…
  ['1pm - 2:30 Lunch', [hm(13), hm(14, 30), 'Lunch']],
  // …else it is 24-hour…
  ['11am - 12:30 Brunch', [hm(11), hm(12, 30), 'Brunch']],
  // …and an end at or before the start is no end.
  ['11pm - 12:30 Late', [hm(23), undefined, 'Late']],
  ['10:00 - 09:00 Backwards', [hm(10), undefined, 'Backwards']],
  // A start without am/pm takes the end's when that keeps it first.
  ['9:30-10:30pm Show', [hm(21, 30), hm(22, 30), 'Show']],
  // Negatives.
  ['2026 goals', null],
  ['10 reasons to stay', null],
  ['15 things', null],
  ['1:1 with Bob', null],
  ['10:30am-ish maybe', null],
  ['9:00', null],
  ['09:00 - 10:00', null],
  ['09:00 -', null],
  ['25:00 Nope', null],
  ['12:60 Nope', null],
  ['13pm Nope', null],
  ['0am Nope', null],
  ['9amigos', null],
  ['noonish Lunch', null],
  ['Standup at 09:00', null],
  ['100:00 Big', null],
  ['', null],
]

describe('parseAgendaText', () => {
  for (const [line, expected] of GRAMMAR) {
    it(`${JSON.stringify(line)} → ${expected ? `${formatMinutes(expected[0])}${expected[1] === undefined ? '' : `–${formatMinutes(expected[1])}`} ${expected[2]}` : 'not an entry'}`, () => {
      const parsed = parseAgendaText(line)
      if (!expected) {
        expect(parsed).toBeNull()
        return
      }
      expect(parsed).not.toBeNull()
      expect(parsed!.start).toBe(expected[0])
      expect(parsed!.end).toBe(expected[1])
      expect(parsed!.title).toBe(expected[2])
    })
  }

  it('reports where the time sits, for the chip', () => {
    const line = 'at 09:00 - 09:15 Standup'
    const parsed = parseAgendaText(line)!
    expect(line.slice(parsed.timeFrom, parsed.timeTo)).toBe('09:00 - 09:15')
    const single = parseAgendaText('  7pm Dinner')!
    expect('  7pm Dinner'.slice(single.timeFrom, single.timeTo)).toBe('7pm')
  })

  it('is matched by the prefilter whenever it parses (the prefilter is a superset)', () => {
    for (const [line, expected] of GRAMMAR) {
      if (expected) expect(AGENDA_PREFILTER.test(line), line).toBe(true)
    }
  })
})

/** pulldown-cmark's rendering of the owner's `## Calendar` section. */
const OWNER_SAMPLE = `<h2 id="calendar">Calendar</h2>
<ul>
<li>09:00 - 09:15 Standup
<ul>
<li>Employees, Bob Wall, Patrick Walsh</li>
<li>Slack call / Google Hangout</li>
</ul>
</li>
<li>10:30 - 11:00 Bob / Patrick 1:1
<ul>
<li>Patrick Walsh, Bob Wall</li>
<li>Hangouts</li>
</ul>
</li>
<li>14:00 - 15:00 Things we did / Rev team demos / experiment results / kudos</li>
</ul>`

function main(html: string): HTMLElement {
  const el = document.createElement('main')
  el.id = 'wrapper'
  el.innerHTML = html
  return el
}

describe('agendaEntries', () => {
  it("reads the owner's daily note: times, titles and first-level details", () => {
    const entries = agendaEntries(main(OWNER_SAMPLE))
    expect(entries.map((e) => [e.ordinal, e.start, e.end, e.title, e.details])).toEqual([
      [1, hm(9), hm(9, 15), 'Standup', ['Employees, Bob Wall, Patrick Walsh', 'Slack call / Google Hangout']],
      [2, hm(10, 30), hm(11), 'Bob / Patrick 1:1', ['Patrick Walsh, Bob Wall', 'Hangouts']],
      [3, hm(14), hm(15), 'Things we did / Rev team demos / experiment results / kudos', []],
    ])
    expect(entries.every((e) => e.task === undefined)).toBe(true)
  })

  it('maps the time range onto the text node, for the chip', () => {
    const [first] = agendaEntries(main(OWNER_SAMPLE))
    const range = first.timeRange!
    expect(range.startContainer).toBe(range.endContainer)
    expect(range.startContainer.data.slice(range.startOffset, range.endOffset)).toBe('09:00 - 09:15')
  })

  it('reads task items (Day Planner), skipping the checkbox and the chips', () => {
    const root = main(`<ul>
<li><input type="checkbox" class="mbr-task-check" id="mbr-task-3" data-mbr-task-line="3" data-mbr-task-status="open" disabled> <span class="mbr-task-text">10:00 - 10:30 Wake up</span> <span class="mbr-task-tag">#morning</span></li>
<li><input type="checkbox" class="mbr-task-check" data-mbr-task-status="done" checked disabled> <span class="mbr-task-text">11:00 Done thing</span></li>
<li><input type="checkbox" class="mbr-task-check" data-mbr-task-status="canceled" disabled> <span class="mbr-task-text mbr-task-canceled">12:00 Dropped</span></li>
</ul>`)
    const entries = agendaEntries(root)
    expect(entries.map((e) => [e.title, e.task])).toEqual([
      ['Wake up', 'open'],
      ['Done thing', 'done'],
      ['Dropped', 'canceled'],
    ])
    const range = entries[0].timeRange!
    expect(range.startContainer.data.slice(range.startOffset, range.endOffset)).toBe('10:00 - 10:30')
  })

  it('reads loose items through their first paragraph only', () => {
    const root = main(`<ul>
<li>
<p>9am Breakfast</p>
<p>17:00 is not the start of this item</p>
</li>
<li>
<p>Notes</p>
<p>18:00 not first either</p>
</li>
</ul>`)
    expect(agendaEntries(root).map((e) => [e.start, e.title])).toEqual([[hm(9), 'Breakfast']])
  })

  it('takes only the first line of an item', () => {
    const root = main('<ul><li>Shopping list\n09:00 is on the second line</li></ul>')
    expect(agendaEntries(root)).toEqual([])
  })

  it('reads inline markup in the leading line', () => {
    const root = main('<ul><li><strong>09:00</strong> - 10:00 <em>Deep</em> work</li></ul>')
    const [entry] = agendaEntries(root)
    expect([entry.start, entry.end, entry.title]).toEqual([hm(9), hm(10), 'Deep work'])
    expect(entry.timeRange!.startContainer.data).toBe('09:00')
  })

  it('ignores nested lists, lists in blockquotes, tables, details and chat bubbles', () => {
    const root = main(`
<ul><li>Projects<ul><li>09:00 Nested is a detail, not an entry</li></ul></li></ul>
<blockquote><ul><li>09:00 Quoted</li></ul></blockquote>
<table><tbody><tr><td><ul><li>09:00 In a table</li></ul></td></tr></tbody></table>
<details><summary>More</summary><ul><li>09:00 Folded</li></ul></details>
<div class="mbr-chat"><div class="mbr-chat-bubble"><ul><li>09:00 Chat</li></ul></div></div>
<dl><dt>Term</dt><dd><ul><li>09:00 In a definition</li></ul></dd></dl>
<section><ul><li>10:00 In a section is still top-level</li></ul></section>`)
    expect(agendaEntries(root).map((e) => e.title)).toEqual(['In a section is still top-level'])
  })

  it('numbers entries in document order across lists', () => {
    const root = main('<ul><li>09:00 A</li><li>Not one</li></ul><p>x</p><ol><li>10:00 B</li></ol>')
    expect(agendaEntries(root).map((e) => [e.ordinal, e.title])).toEqual([
      [1, 'A'],
      [2, 'B'],
    ])
  })
})

describe('leadingText', () => {
  it('stops at a <br>', () => {
    const li = document.createElement('li')
    li.innerHTML = '09:00 Call<br>second line'
    expect(leadingText(li).text).toBe('09:00 Call')
  })

  it('reads a line wrapped in span.mbr-agenda-line as it read the bare one', () => {
    const html = '09:00 - 09:15 Standup\n<ul>\n<li>Bob</li>\n</ul>\n'
    const bare = document.createElement('li')
    bare.innerHTML = html
    const wrapped = document.createElement('li')
    wrapped.innerHTML = html
    const span = document.createElement('span')
    span.className = 'mbr-agenda-line'
    span.append(wrapped.firstChild!)
    wrapped.prepend(span)
    expect(leadingText(wrapped).text).toBe(leadingText(bare).text)
    expect(leadingText(wrapped).text.trim()).toBe('09:00 - 09:15 Standup')
    expect(leadingText(wrapped).segments[0].node).toBe(span.firstChild)
  })
})
