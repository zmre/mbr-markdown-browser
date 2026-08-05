import { describe, it, expect, afterEach } from 'vitest'
import { doneChipFor, syncDoneChip } from './task-chips.js'

describe('doneChipFor', () => {
  it('renders a bare date the way `push_task_time` does', () => {
    expect(doneChipFor('- [x] ship it @done(2026-08-04)')).toEqual({
      datetime: '2026-08-04',
      label: 'Aug 4',
    })
  })

  it('renders the stamp the server actually writes', () => {
    // `tasks::DONE_STAMP_FORMAT` is `%Y-%m-%d %H:%M`, and `push_task_time`
    // shows it as `%b %-d, %-I:%M %p` — no leading zero on the hour, a plain
    // space before AM/PM (which is why this is hand-formatted rather than
    // `toLocaleTimeString`, whose en-US output uses U+202F).
    expect(doneChipFor('- [x] ship it @done(2026-08-04 22:16)')).toEqual({
      datetime: '2026-08-04T22:16',
      label: 'Aug 4, 10:16 PM',
    })
    expect(doneChipFor('- [x] a @done(2026-01-31 09:05)')).toEqual({
      datetime: '2026-01-31T09:05',
      label: 'Jan 31, 9:05 AM',
    })
  })

  it('puts midnight and noon on the 12-hour clock the way chrono does', () => {
    expect(doneChipFor('- [x] a @done(2026-08-04 00:00)')?.label).toBe('Aug 4, 12:00 AM')
    expect(doneChipFor('- [x] a @done(2026-08-04 12:00)')?.label).toBe('Aug 4, 12:00 PM')
  })

  it('accepts the hand-written 12-hour forms `tasks::parse_datetime` accepts', () => {
    expect(doneChipFor('- [x] a @done(2026-08-04 3:00 PM)')).toEqual({
      datetime: '2026-08-04T15:00',
      label: 'Aug 4, 3:00 PM',
    })
    // Case-insensitive, and the space before the meridiem is optional.
    expect(doneChipFor('- [x] a @done(2026-08-04 12:30am)')?.datetime).toBe('2026-08-04T00:30')
  })

  it('has no chip for a task with no stamp', () => {
    expect(doneChipFor('- [ ] ship it !! #work @due(2026-08-05)')).toBeNull()
    expect(doneChipFor('- [-] canceled')).toBeNull()
  })

  it('ignores a `@done(...)` whose payload is not a datetime', () => {
    // Not an annotation on the Rust side either: it stays in the display text
    // rather than becoming a chip, so drawing one here would be a lie.
    for (const payload of ['soon', '2026-13-45', '2026-02-30', '', '2026-08-04 25:00']) {
      expect(doneChipFor(`- [x] a @done(${payload})`)).toBeNull()
    }
    // `14:00 PM` is not a time anybody meant (`tasks::parse_datetime`).
    expect(doneChipFor('- [x] a @done(2026-08-04 14:00 PM)')).toBeNull()
  })

  it('takes the first parseable stamp, matching `strip_annotations`', () => {
    expect(doneChipFor('- [x] a @done(nope) @done(2026-08-04) @done(2020-01-01)')).toEqual({
      datetime: '2026-08-04',
      label: 'Aug 4',
    })
  })

  it('is not confused by state left on the global regex', () => {
    // `DONE_ANNOTATION` is a `/g` regex at module scope; a stale `lastIndex`
    // would make every other call miss.
    const line = '- [x] a @done(2026-08-04)'
    expect(doneChipFor(line)).toEqual(doneChipFor(line))
    expect(doneChipFor(line)).not.toBeNull()
  })
})

describe('syncDoneChip', () => {
  /**
   * One rendered task, exactly as `markdown.rs` assembles it: the checkbox,
   * the text span, then the chips in `task_annotations_html` order, each
   * preceded by a single space.
   */
  function render(chips: string): HTMLInputElement {
    const item = document.createElement('li')
    item.innerHTML =
      `<input type="checkbox" class="mbr-task-check" id="mbr-task-3" ` +
      `data-mbr-task-line="3" data-mbr-task-status="open">` +
      `<span class="mbr-task-text">write the report</span>${chips}`
    document.body.appendChild(item)
    return item.querySelector('input')!
  }

  /** Everything after the checkbox, which is what the renderer would produce. */
  function markup(input: HTMLInputElement): string {
    return input.parentElement!.innerHTML.slice(input.outerHTML.length)
  }

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('adds the chip a freshly stamped line calls for', () => {
    const input = render('')

    syncDoneChip(input, '- [x] write the report @done(2026-08-04 22:16)')

    expect(markup(input)).toBe(
      '<span class="mbr-task-text">write the report</span>' +
        ' <time class="mbr-task-completed" datetime="2026-08-04T22:16">Aug 4, 10:16 PM</time>'
    )
  })

  it('keeps the chip order `task_annotations_html` emits', () => {
    const input = render(
      ' <span class="mbr-task-pri mbr-task-pri-high" role="img"></span>' +
        ' <span class="mbr-task-tag">#work</span>' +
        ' <time class="mbr-task-due" datetime="2026-08-05">Aug 5</time>' +
        ' <time class="mbr-task-moved" datetime="2026-09-01">Sep 1</time>'
    )

    syncDoneChip(input, '- [x] write the report @done(2026-08-04)')

    // Completion sits after due and before the move marker.
    const classes = Array.from(input.parentElement!.querySelectorAll('span, time'))
      .map((el) => el.className)
      .filter((name) => name !== 'mbr-task-text')
    expect(classes).toEqual([
      'mbr-task-pri mbr-task-pri-high',
      'mbr-task-tag',
      'mbr-task-due',
      'mbr-task-completed',
      'mbr-task-moved',
    ])
    // ...separated by exactly one space, like every other chip.
    expect(markup(input)).toContain(
      '<time class="mbr-task-due" datetime="2026-08-05">Aug 5</time>' +
        ' <time class="mbr-task-completed" datetime="2026-08-04">Aug 4</time>' +
        ' <time class="mbr-task-moved" datetime="2026-09-01">Sep 1</time>'
    )
  })

  it('removes the chip, and its space, when a task is reopened', () => {
    const before = '<span class="mbr-task-text">write the report</span>'
    const input = render(' <time class="mbr-task-completed" datetime="2026-08-04">Aug 4</time>')

    syncDoneChip(input, '- [ ] write the report')

    // Byte-identical to a task that never had a stamp: no orphan whitespace.
    expect(markup(input)).toBe(before)
  })

  it('leaves the other chips alone when it removes the completion one', () => {
    const input = render(
      ' <span class="mbr-task-tag">#work</span>' +
        ' <time class="mbr-task-completed" datetime="2026-08-04">Aug 4</time>' +
        ' <time class="mbr-task-moved" datetime="2026-09-01">Sep 1</time>'
    )

    syncDoneChip(input, '- [ ] write the report #work > 2026-09-01')

    expect(markup(input)).toBe(
      '<span class="mbr-task-text">write the report</span>' +
        ' <span class="mbr-task-tag">#work</span>' +
        ' <time class="mbr-task-moved" datetime="2026-09-01">Sep 1</time>'
    )
  })

  it('updates a stamp in place rather than adding a second chip', () => {
    const input = render(' <time class="mbr-task-completed" datetime="2026-08-04">Aug 4</time>')

    syncDoneChip(input, '- [x] write the report @done(2026-08-06 09:30)')

    expect(input.parentElement!.querySelectorAll('.mbr-task-completed')).toHaveLength(1)
    expect(markup(input)).toBe(
      '<span class="mbr-task-text">write the report</span>' +
        ' <time class="mbr-task-completed" datetime="2026-08-06T09:30">Aug 6, 9:30 AM</time>'
    )
  })

  it('is idempotent, so a repeated write cannot pile chips up', () => {
    const input = render('')
    const line = '- [x] write the report @done(2026-08-04)'

    syncDoneChip(input, line)
    const once = markup(input)
    syncDoneChip(input, line)

    expect(markup(input)).toBe(once)
  })

  it('does not reach past the chips into a nested subtask list', () => {
    const input = render(
      '<ul><li><input type="checkbox" class="mbr-task-check" id="mbr-task-4">' +
        '<span class="mbr-task-text">subtask</span></li></ul>'
    )

    syncDoneChip(input, '- [x] write the report @done(2026-08-04)')

    // The chip goes before the nested list, not inside or after it.
    const parentChildren = Array.from(input.parentElement!.children).map((el) => el.tagName)
    expect(parentChildren).toEqual(['INPUT', 'SPAN', 'TIME', 'UL'])
    expect(input.parentElement!.querySelectorAll('.mbr-task-completed')).toHaveLength(1)
  })

  it('no-ops on markup with no task text span', () => {
    const item = document.createElement('li')
    item.innerHTML = '<input type="checkbox" class="mbr-task-check">'
    document.body.appendChild(item)
    const input = item.querySelector('input')!

    expect(() => syncDoneChip(input, '- [x] a @done(2026-08-04)')).not.toThrow()
    expect(item.querySelector('.mbr-task-completed')).toBeNull()
  })
})
