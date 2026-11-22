/**
 * `<mbr-calendar-overlay>` — the calendar overlay (calendar chunk;
 * design/calendar.md §5): a full-screen Month / Week / List view of a set of
 * notes, plotted by note date, modified or created, with a type-to-filter
 * field, a jump picker and its own keys.
 *
 * `<mbr-calendar>` (main bundle) renders it once first asked and keeps it, so
 * the view, plot-by and filter persist for the life of the page; it hands over
 * `scope` (`CalendarScope`: the notes, or all of them) and the services the
 * chunk may not import (`siteIndex`, `resolveUrl`, `navigate`, `config`,
 * `tagFields`, and `taskSource` where there is a task endpoint). Notes come
 * from `site.json` alone, so it works the same in static builds; with a
 * `taskSource` (server/GUI with `tasks_enabled`) it also places tasks on
 * their days — due, scheduled or done — from `POST /.mbr/tasks`
 * `mode: "grid"`, one request per range on screen.
 *
 * The toolbar keeps the filter field, the active-scope chips and the count;
 * scope ("This folder"), plot-by and tasks are in the options menu (`o`).
 *
 * The model is `overlay-model.ts` (bucketing once per scope and plot-by, the
 * filter) and `month-grid.ts` (the key map, ranges and titles); the grids are
 * `mbr-calendar-grid.ts`. Nothing renders more than it can show: the grids
 * draw what fits, the day popover and each List day stop at a cap with a
 * `+N more`, so a scope where thousands of notes share a day (Plot by
 * modified on a fresh clone) stays fast. While open, one
 * capture-phase `window` keydown listener owns the keyboard (the flashcard
 * deck's pattern): it stops every key's propagation, so the page's `[`, `]`,
 * `p` and `?` never see them, and prevents the default only for keys it
 * handles, so typing in the filter and activating a focused control still work.
 */
import { LitElement, css, html, nothing, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { positionAnchored } from '../anchored-popover.js'
import { ICON_BUTTON_CSS, KBD_CSS } from '../controls.js'
import {
  ICON_SM,
  icon,
  iconChevronDown,
  iconChevronLeft,
  iconChevronRight,
  iconCircleHelp,
  iconSlidersHorizontal,
  iconX,
} from '../icons.js'
import { folderScopeValue } from '../tasks/folder-tree.js'
import { gridHiddenCount, gridRange } from '../tasks/task-grid.js'
import { taskHref } from '../tasks/task-groups.js'
import { taskDisplayText } from '../tasks/task-text.js'
import type { GridTaskHit, Placement, TaskGridResponse } from '../tasks/types.js'
import type { MarkdownFile } from '../sorting.js'
import { DEFAULT_CALENDAR_CONFIG, type CalendarConfig } from './config.js'
import { formatClock, formatDate, isoWeekOf, monthOf, resolveWeekStart, viewTitle, weekNumberLabel } from './month-grid.js'
import { addDays, todayKey, type DateKey } from './note-date.js'
import { periodKeysContaining } from './period.js'
import {
  OVERLAY_SHORTCUTS,
  PLOT_BYS,
  PLOT_BY_LABELS,
  DEFAULT_TASKS_SHOW,
  TASKS_SHOWS,
  TASKS_SHOW_LABELS,
  bucketNotes,
  daysBetween,
  defaultView,
  filesInFolder,
  matchesFilter,
  moveDay,
  nextMatch,
  nextPlotBy,
  overflowSplit,
  prevMatch,
  overlayCommand,
  pageFolder,
  parseFilter,
  parseJump,
  taskMatchesFilter,
  taskMinutes,
  tasksStatuses,
  viewRange,
  type Buckets,
  type CalendarView,
  type OverlayCommand,
  type OverlayEntry,
  type TasksShow,
} from './overlay-model.js'
import { findPeriod, periodicIndexOf, type PeriodicIndex } from './periodic.js'
import {
  OVERLAY_CLOSE_EVENT,
  type CalendarScope,
  type DayMarks,
  type GridEntry,
  type PlotBy,
  type SiteIndex,
  type TaskGridSource,
} from './types.js'
import './mbr-calendar-grid.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-calendar-overlay': MbrCalendarOverlayElement
  }
}

/** Filter debounce (§5.3, the media browser's). */
const FILTER_DEBOUNCE_MS = 100

/** Most rows the day popover lists before its `+N more`. */
const POPOVER_CAP = 100
/** Most rows one List day shows before its `+N more`. */
const LIST_DAY_CAP = 50

/** One day as the views draw it, after the filter. */
interface DayView {
  readonly daily: OverlayEntry | null
  /** The day's notes (the day note aside). */
  readonly entries: readonly OverlayEntry[]
  /** The day's tasks in scope that pass the filter, in the server's order (open first). */
  readonly tasks: readonly GridTaskHit[]
  /** Tasks the day has but the server's row limit dropped (counted only without a filter). */
  readonly hiddenTasks: number
  /** Tasks in scope before the filter, the dropped ones included. */
  readonly taskTotal: number
  /** A past day with an open task due on it (the server's flag). */
  readonly overdue: boolean
}

const EMPTY_DAY: DayView = Object.freeze({ daily: null, entries: [], tasks: [], hiddenTasks: 0, taskTotal: 0, overdue: false })

/** One line of a day in the popover and the List: a note or a task. */
type DayItem = { readonly kind: 'note'; readonly entry: OverlayEntry } | { readonly kind: 'task'; readonly hit: GridTaskHit }

/** Everything a day lists, day note first, then notes, then tasks. */
function dayItemCount(day: DayView): number {
  return (day.daily ? 1 : 0) + day.entries.length + day.tasks.length
}

/** The first `cap` lines of a day, without building the rest. */
function dayItems(day: DayView, cap: number): DayItem[] {
  const out: DayItem[] = day.daily ? [{ kind: 'note', entry: day.daily }] : []
  for (const entry of day.entries) {
    if (out.length >= cap) return out
    out.push({ kind: 'note', entry })
  }
  for (const hit of day.tasks) {
    if (out.length >= cap) return out
    out.push({ kind: 'task', hit })
  }
  return out
}

/** Commands that move the focus into the grid: not taken from inside the options menu. */
const MOVES: ReadonlySet<OverlayCommand> = new Set(['left', 'right', 'up', 'down', 'prev', 'next', 'open'])

/** Why a task sits on its day, for its tooltip and its List / popover row. */
const PLACED_LABELS: Readonly<Record<Placement, string>> = { due: 'Due', scheduled: 'Scheduled', done: 'Done' }

/** The List view's rows, and how many each capped day left out. */
interface ListRows {
  readonly rows: readonly { readonly date: DateKey; readonly item: DayItem }[]
  readonly more: ReadonlyMap<DateKey, number>
}

/** The grids' per-day callbacks, rebuilt (new identities) whenever the day data changes. */
interface GridCallbacks {
  readonly gen: number
  readonly month: (date: DateKey, limit: number) => readonly GridEntry[]
  readonly week: (date: DateKey, limit: number) => readonly GridEntry[]
  readonly count: (date: DateKey) => number
  readonly marks: (date: DateKey) => DayMarks
}

@customElement('mbr-calendar-overlay')
export class MbrCalendarOverlayElement extends LitElement {
  @property({ type: Boolean, reflect: true }) open = false
  /** What the overlay plots; a new object re-scopes it. */
  @property({ attribute: false }) scope: CalendarScope = { label: 'All notes' }
  @property({ attribute: false }) siteIndex: () => Promise<SiteIndex | null> = () => Promise.resolve(null)
  @property({ attribute: false }) resolveUrl: (path: string) => string = (path) => path
  @property({ attribute: false }) navigate: (url: string) => void = (url) => window.location.assign(url)
  @property({ attribute: false }) config: CalendarConfig = DEFAULT_CALENDAR_CONFIG
  /** Frontmatter fields holding tags (the configured tag sources). */
  @property({ attribute: false }) tagFields: readonly string[] = ['tags']
  @property() locale = typeof navigator === 'undefined' ? 'en-US' : navigator.language || 'en-US'
  @property({ attribute: false }) now: () => Date = () => new Date()
  /**
   * Task rows for the range on screen (`calendar/task-markers.ts`); `null`
   * where there is no task endpoint (static builds, `tasks_enabled = false`):
   * no Tasks option, no request. A new identity means the rows went stale.
   */
  @property({ attribute: false }) taskSource: TaskGridSource | null = null

  @state() private _all: readonly MarkdownFile[] | null = null
  @state() private _label = 'All notes'
  /** The scope's own notes; `null` = all. */
  @state() private _scoped: readonly MarkdownFile[] | null = null
  /** The page's folder when opened with `c` from a page ('' = none). */
  @state() private _folder = ''
  @state() private _folderOnly = false
  @state() private _view: CalendarView | null = null
  @state() private _plotBy: PlotBy = 'date'
  @state() private _filterText = ''
  @state() private _terms: ReturnType<typeof parseFilter> = []
  @state() private _focus: DateKey = ''
  @state() private _today: DateKey = ''
  @state() private _listIndex = 0
  @state() private _popover: { date: DateKey; anchor: HTMLElement } | null = null
  @state() private _picker = false
  @state() private _pickerYear = 2026
  @state() private _jumpError = false
  @state() private _help = false
  @state() private _options = false
  /** Which tasks to show beside the notes, kept for the life of the page. */
  @state() private _tasksShow: TasksShow = DEFAULT_TASKS_SHOW
  /** The task rows of the range on screen; `null` while none are wanted or loaded. */
  @state() private _tasks: TaskGridResponse | null = null

  private _buckets: Buckets | null = null
  private _bucketsKey: readonly [readonly MarkdownFile[], boolean, PlotBy] | null = null
  private _dayCache = new Map<DateKey, DayView>()
  /** Bumped whenever `_dayCache` is dropped: keys the memos below. */
  private _dayGen = 0
  private _listMemo: { key: string; value: ListRows } | null = null
  private _matchMemo = new Map<string, DateKey | null>()
  private _debounce: number | null = null
  private _focusAfterRender: 'grid' | 'popover' | 'list' | 'options' | 'options-button' | null = null
  private _gridCallbacks: GridCallbacks | null = null
  /** The task request in flight or answered: its query key and the source asked. */
  private _tasksAsked: { key: string; source: TaskGridSource } | null = null
  private _scopedUrls: { files: readonly MarkdownFile[]; urls: ReadonlySet<string> } | null = null

  override connectedCallback(): void {
    super.connectedCallback()
    if (this.open) window.addEventListener('keydown', this._onKeydown, true)
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    window.removeEventListener('keydown', this._onKeydown, true)
    if (this._debounce !== null) window.clearTimeout(this._debounce)
  }

  /** Re-scopes to all notes (the scope chip's ✕). */
  public widen(): void {
    this._scoped = null
    this._label = 'All notes'
    this._folderOnly = false
  }

  protected override willUpdate(changed: PropertyValues<this>): void {
    if (changed.has('open')) {
      if (this.open) window.addEventListener('keydown', this._onKeydown, true)
      else window.removeEventListener('keydown', this._onKeydown, true)
    }
    if ((changed.has('open') || changed.has('scope')) && this.open) this._applyScope()
    this._syncTasks()
  }

  /** Whether tasks are on screen: an endpoint and a Tasks choice other than None. */
  private get _tasksOn(): boolean {
    return this.taskSource !== null && this._tasksShow !== 'none'
  }

  /**
   * Asks for the tasks of the range on screen when it, the statuses, the
   * folder or the source changed: once per change, never while closed. A new
   * range drops the old rows at once; a new source (a file changed) keeps
   * them on screen until the fresh ones arrive.
   */
  private _syncTasks(): void {
    if (!this.open || !this._focus) return
    const source = this._tasksOn ? this.taskSource : null
    if (!source) {
      if (this._tasks || this._tasksAsked) {
        this._tasks = null
        this._tasksAsked = null
        this._dropDays()
      }
      return
    }
    const view = this._view ?? 'month'
    const weekStart = this._weekStart()
    const { from, to } = view === 'list' ? viewRange('list', this._focus, weekStart) : gridRange(view, this._focus, weekStart)
    const statuses = tasksStatuses(this._tasksShow)
    const folder = this._folderOnly && this._folder ? folderScopeValue(this._folder) : null
    const key = `${from}|${to}|${statuses.join(',')}|${folder ?? ''}`
    const asked = this._tasksAsked
    if (asked?.key === key && asked.source === source) return
    if (asked?.key !== key && this._tasks) {
      this._tasks = null
      this._dropDays()
    }
    const ask = { key, source }
    this._tasksAsked = ask
    source({ from, to, statuses, folder })
      .then((response) => {
        if (this._tasksAsked !== ask) return
        this._tasks = response
        this._dropDays()
      })
      .catch((err: unknown) => {
        if (this._tasksAsked === ask) this._tasksAsked = null
        console.warn('mbr-calendar: could not load tasks', err)
      })
  }

  protected override updated(): void {
    const target = this._focusAfterRender
    if (!target) return
    this._focusAfterRender = null
    if (target === 'popover') {
      const card = this.renderRoot.querySelector<HTMLElement>('.popover')
      if (card && this._popover) {
        positionAnchored(this._popover.anchor, card)
        card.querySelector<HTMLElement>('a')?.focus()
      }
    } else if (target === 'options') {
      const card = this.renderRoot.querySelector<HTMLElement>('.options')
      const button = this.renderRoot.querySelector<HTMLElement>('.options-btn')
      if (card && button) {
        positionAnchored(button, card)
        const radio = card.querySelector<HTMLElement>('input:checked') ?? card.querySelector<HTMLElement>('input')
        radio?.focus()
      }
    } else if (target === 'options-button') {
      this.renderRoot.querySelector<HTMLElement>('.options-btn')?.focus()
    } else if (target === 'list' || this._view === 'list') {
      this.renderRoot.querySelector<HTMLElement>(`.row[data-i="${this._listIndex}"]`)?.focus()
    } else {
      void this._grid()?.focusDay()
    }
  }

  /** Opening (or a new scope while open): label, plot-by, starting day, data. */
  private _applyScope(): void {
    const scope = this.scope
    this._label = scope.label
    this._scoped = scope.files ?? null
    this._folderOnly = false
    this._folder = ''
    if (scope.plotBy) this._plotBy = scope.plotBy
    this._view ??= defaultView(window.innerWidth)
    this._today = todayKey(this.now())
    this._focus = this._today
    this._listIndex = 0
    this._popover = null
    this._picker = false
    this._help = false
    this._options = false
    this._focusAfterRender = 'grid'
    const ready = (files: readonly MarkdownFile[]): void => {
      this._all = files
      if (scope.pagePath) this._folder = pageFolder(files, scope.pagePath)
      this._startNearMatches()
      this._focusAfterRender = this._view === 'list' ? 'list' : 'grid'
    }
    if (this._all) ready(this._all)
    else
      void this.siteIndex()
        .then((site) => ready(site?.markdown_files ?? []))
        .catch((err: unknown) => console.warn('mbr-calendar: could not load the site index', err))
  }

  /** A scope with nothing this month opens on the nearest month that has something. */
  private _startNearMatches(): void {
    const buckets = this._bucketsNow()
    if (!buckets || !this._scoped) return
    const { from, to } = viewRange(this._view ?? 'month', this._focus, this._weekStart())
    if (this._countRange(from, to).shown > 0) return
    const near = this._nearMatch(-1, from, to) ?? this._nearMatch(1, from, to)
    if (near) this._focus = near
  }

  private _grid(): HTMLElementTagNameMap['mbr-calendar-month-grid'] | HTMLElementTagNameMap['mbr-calendar-week-grid'] | null {
    return this.renderRoot.querySelector('mbr-calendar-month-grid') ?? this.renderRoot.querySelector('mbr-calendar-week-grid')
  }

  private _weekStart() {
    return resolveWeekStart(this.config.weekStart, this.locale)
  }

  private get _index(): PeriodicIndex | null {
    return this._all ? periodicIndexOf(this._all) : null
  }

  /** The files in scope now (after "This folder"). */
  private _scopeFiles(): readonly MarkdownFile[] | null {
    const files = this._scoped ?? this._all
    if (!files) return null
    return this._folderOnly && this._folder ? filesInFolder(files, this._folder) : files
  }

  /** The buckets for the current scope and plot-by, built once per pair (§5.4). */
  private _bucketsNow(): Buckets | null {
    const base = this._scoped ?? this._all
    if (!base) return null
    const k = this._bucketsKey
    if (!this._buckets || !k || k[0] !== base || k[1] !== this._folderOnly || k[2] !== this._plotBy) {
      this._buckets = bucketNotes(this._scopeFiles()!, this._plotBy, {
        tagFields: this.tagFields,
        dateFields: this.config.dateFields,
      })
      this._bucketsKey = [base, this._folderOnly, this._plotBy]
      this._dropDays()
    }
    return this._buckets
  }

  /** Forgets every filtered day and what was derived from them. */
  private _dropDays(): void {
    this._dayCache.clear()
    this._dayGen++
    this._listMemo = null
    this._matchMemo.clear()
  }

  /**
   * The nearest day with a match before `from` (`-1`) or after `to` (`1`),
   * memoized per (buckets, filter, range): render asks on every keystroke.
   */
  private _nearMatch(dir: -1 | 1, from: DateKey, to: DateKey): DateKey | null {
    const buckets = this._bucketsNow()
    if (!buckets) return null
    const key = `${dir}|${from}|${to}`
    if (!this._matchMemo.has(key)) {
      this._matchMemo.set(key, dir < 0 ? prevMatch(buckets, from, this._terms) : nextMatch(buckets, to, this._terms))
    }
    return this._matchMemo.get(key)!
  }

  /** The url_paths of an explicit scope's notes, which its tasks must come from. */
  private _scopedUrlSet(): ReadonlySet<string> | null {
    const files = this._scoped
    if (!files) return null
    if (this._scopedUrls?.files !== files) this._scopedUrls = { files, urls: new Set(files.map((f) => f.url_path)) }
    return this._scopedUrls.urls
  }

  /**
   * One day after the filter, memoized until the filter, buckets or tasks
   * change. Tasks are in scope when their note is (an explicit scope is
   * matched here; "This folder" was the request's own folder filter).
   */
  private _day(date: DateKey): DayView {
    const buckets = this._bucketsNow()
    const all = buckets?.days.get(date)
    const taskDay = this._tasks?.days[date]
    if (!all && !taskDay) return EMPTY_DAY
    let view = this._dayCache.get(date)
    if (!view) {
      const kept = !all ? [] : this._terms.length ? all.filter((e) => matchesFilter(e, this._terms)) : all
      const daily = kept[0]?.daily ? kept[0] : null
      const urls = this._scopedUrlSet()
      const inScope = taskDay ? (urls ? taskDay.tasks.filter((h) => urls.has(h.url_path)) : taskDay.tasks) : []
      // The rows the server's limit dropped are counted only where its counts
      // describe the scope (no explicit scope) and no filter narrows them.
      const dropped = taskDay && !urls ? gridHiddenCount(taskDay, tasksStatuses(this._tasksShow)) : 0
      const tasks = this._terms.length ? inScope.filter((h) => taskMatchesFilter(h, this._terms)) : inScope
      const hiddenTasks = this._terms.length ? 0 : dropped
      view = {
        daily,
        entries: daily ? kept.slice(1) : kept,
        tasks,
        hiddenTasks,
        taskTotal: inScope.length + dropped,
        overdue: (taskDay?.overdue ?? false) && tasks.length + hiddenTasks > 0,
      }
      this._dayCache.set(date, view)
    }
    return view
  }

  /** Notes (shown of total) and tasks (shown of total) across `from`…`to`. */
  private _countRange(from: DateKey, to: DateKey): { shown: number; total: number; tasksShown: number; taskTotal: number } {
    let shown = 0
    let total = 0
    let tasksShown = 0
    let taskTotal = 0
    const buckets = this._bucketsNow()
    if (!buckets) return { shown, total, tasksShown, taskTotal }
    for (const date of daysBetween(from, to)) {
      const all = buckets.days.get(date)
      if (!all && !this._tasks?.days[date]) continue
      total += all?.length ?? 0
      const day = this._day(date)
      shown += day.entries.length + (day.daily ? 1 : 0)
      tasksShown += day.tasks.length + day.hiddenTasks
      taskTotal += day.taskTotal
    }
    return { shown, total, tasksShown, taskTotal }
  }

  private _gridEntry(e: OverlayEntry, withTime: boolean): GridEntry {
    return {
      title: e.title,
      href: this.resolveUrl(e.file.url_path),
      color: e.color,
      ...(withTime && e.minutes !== null ? { time: formatClock(e.minutes, this.locale) } : {}),
      tooltip: [e.title, e.type, e.folder].filter(Boolean).join(' · '),
    }
  }

  /** A task as a grid line: its checkbox glyph, its text, a link to its line, why it sits there on hover. */
  private _taskEntry(hit: GridTaskHit, withTime: boolean): GridEntry {
    const text = taskDisplayText(hit.text)
    const minutes = withTime ? taskMinutes(hit.time) : null
    return {
      title: text,
      href: taskHref(hit, this.resolveUrl),
      status: hit.status,
      ...(minutes !== null ? { time: formatClock(minutes, this.locale) } : {}),
      tooltip: [text, PLACED_LABELS[hit.placed_by], hit.note].join(' · '),
    }
  }

  /** The first `limit` lines of a day — notes, then tasks: a grid never asks for more than it draws. */
  private _dayGridEntries(date: DateKey, limit: number, withTime: boolean): GridEntry[] {
    const day = this._day(date)
    const notes = day.entries.slice(0, limit).map((e) => this._gridEntry(e, withTime))
    const room = limit - notes.length
    return room > 0 ? [...notes, ...day.tasks.slice(0, room).map((h) => this._taskEntry(h, withTime))] : notes
  }

  /**
   * The grids' callbacks, with new identities whenever the day data changes
   * (`_dayGen`): a grid re-renders only when one of its properties changes,
   * and the same functions over new buckets (a filter, "This folder", plot-by,
   * tasks arriving) would leave every cell as it was. Kept between renders
   * otherwise, so an unrelated overlay render does not re-render the grid.
   */
  private _callbacks(): GridCallbacks {
    this._bucketsNow()
    const gen = this._dayGen
    if (this._gridCallbacks?.gen !== gen) {
      this._gridCallbacks = {
        gen,
        month: (date, limit) => this._dayGridEntries(date, limit, false),
        week: (date, limit) => this._dayGridEntries(date, limit, true),
        count: (date) => {
          const day = this._day(date)
          return day.entries.length + day.tasks.length
        },
        marks: (date) => {
          const day = this._day(date)
          const marks: { -readonly [K in keyof DayMarks]: DayMarks[K] } = { notes: day.entries.length }
          if (day.daily) marks.dailyUrl = this.resolveUrl(day.daily.file.url_path)
          if (day.tasks.length || day.hiddenTasks) {
            marks.open = day.tasks.filter((h) => h.status === 'open').length
            marks.hidden = day.hiddenTasks
            marks.overdue = day.overdue
          }
          return marks
        },
      }
    }
    return this._gridCallbacks
  }

  private _itemHref(item: DayItem): string {
    return item.kind === 'task' ? taskHref(item.hit, this.resolveUrl) : this.resolveUrl(item.entry.file.url_path)
  }

  private _periodUrl(key: string): string | null {
    const index = this._index
    const url = index ? findPeriod(index, key)?.url : undefined
    return url ? this.resolveUrl(url) : null
  }

  private _weekHref = (isoWeek: string): string | null => this._periodUrl(isoWeek)

  // ---- actions ----

  private _close(): void {
    this._popover = null
    this._picker = false
    this._help = false
    this._options = false
    this.dispatchEvent(new CustomEvent(OVERLAY_CLOSE_EVENT, { bubbles: true, composed: true }))
  }

  private _setFocus(date: DateKey): void {
    this._focus = date
    this._listIndex = 0
    this._popover = null
    this._focusAfterRender = this._view === 'list' ? 'list' : 'grid'
  }

  private _setView(view: CalendarView): void {
    this._view = view
    this._listIndex = 0
    this._popover = null
    this._focusAfterRender = view === 'list' ? 'list' : 'grid'
  }

  private _setPlotBy(plotBy: PlotBy): void {
    this._plotBy = plotBy
    this._listIndex = 0
    this._popover = null
  }

  private _setFilter(text: string): void {
    this._filterText = text
    if (this._debounce !== null) window.clearTimeout(this._debounce)
    this._debounce = window.setTimeout(() => {
      this._debounce = null
      this._terms = parseFilter(text)
      this._dropDays()
      this._listIndex = 0
    }, FILTER_DEBOUNCE_MS)
  }

  private _setFolderOnly(on: boolean): void {
    this._folderOnly = on
    this._listIndex = 0
    this._popover = null
  }

  private _setTasksShow(show: TasksShow): void {
    this._tasksShow = show
    this._listIndex = 0
    this._popover = null
  }

  /** The options menu (`o`, the sliders button); opening it closes the other popovers. */
  private _toggleOptions(): void {
    if (this._options) {
      this._closeOptions()
      return
    }
    this._options = true
    this._popover = null
    this._picker = false
    this._help = false
    this._focusAfterRender = 'options'
  }

  private _closeOptions(): void {
    this._options = false
    this._focusAfterRender = 'options-button'
  }

  /** A press outside the open options menu (and its button) closes it. */
  private _onDialogPointer = (e: PointerEvent): void => {
    if (!this._options) return
    const inside = e.composedPath().some((n) => n instanceof Element && (n.classList.contains('options') || n.classList.contains('options-btn')))
    if (!inside) this._options = false
  }

  /** Whether any option differs from its default (the button's dot). */
  private _optionsChanged(): boolean {
    return this._folderOnly || this._plotBy !== 'date' || (this.taskSource !== null && this._tasksShow !== DEFAULT_TASKS_SHOW)
  }

  /** Enter on a day: its only note or task opens; several list in the popover. */
  private _openDay(date: DateKey, anchor?: HTMLElement | null): void {
    const day = this._day(date)
    const count = dayItemCount(day)
    if (count === 0) return
    if (count === 1) {
      this.navigate(this._itemHref(dayItems(day, 1)[0]))
      return
    }
    const cell = anchor ?? this._grid()?.cellFor(date)
    if (!cell) return
    this._popover = { date, anchor: cell }
    this._focusAfterRender = 'popover'
  }

  private _onGridDay = (e: CustomEvent<{ date: DateKey; anchor: HTMLElement }>): void => {
    this._focus = e.detail.date
    if (dayItemCount(this._day(e.detail.date)) > 0) {
      this._popover = { date: e.detail.date, anchor: e.detail.anchor }
      this._focusAfterRender = 'popover'
    }
  }

  private _onGridFocus = (e: CustomEvent<{ date: DateKey }>): void => {
    this._focus = e.detail.date
  }

  private _togglePicker(): void {
    this._picker = !this._picker
    if (this._picker) this._options = false
    this._pickerYear = Number(this._focus.slice(0, 4))
    this._jumpError = false
  }

  private _jump(date: DateKey): void {
    this._picker = false
    this._setFocus(date)
  }

  private _onJumpSubmit = (e: Event): void => {
    e.preventDefault()
    const input = (e.currentTarget as HTMLFormElement).querySelector('input')
    const parsed = parseJump(input?.value ?? '')
    if (!parsed) {
      this._jumpError = true
      return
    }
    if (parsed.kind === 'week' && this._view === 'month') this._view = 'week'
    this._jump(parsed.date)
  }

  /**
   * The rows of the List view, flat, each day capped at {@link LIST_DAY_CAP}
   * (the rest is the day's `more`). Memoized per (month, filter, buckets).
   */
  private _listRows(): ListRows {
    const { from, to } = viewRange('list', this._focus, this._weekStart())
    this._bucketsNow()
    const key = `${from}|${this._dayGen}`
    if (this._listMemo?.key === key) return this._listMemo.value
    const rows: { date: DateKey; item: DayItem }[] = []
    const more = new Map<DateKey, number>()
    for (const date of daysBetween(from, to)) {
      const day = this._day(date)
      const items = dayItems(day, LIST_DAY_CAP)
      const split = overflowSplit(items.length, LIST_DAY_CAP, dayItemCount(day) + day.hiddenTasks)
      for (const item of items.slice(0, split.shown)) rows.push({ date, item })
      if (split.more) more.set(date, split.more)
    }
    const value = { rows, more }
    this._listMemo = { key, value }
    return value
  }

  private _run(command: OverlayCommand): boolean {
    const view = this._view ?? 'month'
    switch (command) {
      case 'escape':
        if (this._help) this._help = false
        else if (this._options) this._closeOptions()
        else if (this._popover) {
          this._popover = null
          this._focusAfterRender = 'grid'
        } else if (this._picker) this._picker = false
        else this._close()
        return true
      case 'help':
        this._help = !this._help
        return true
      case 'today':
        this._today = todayKey(this.now())
        this._setFocus(this._today)
        return true
      case 'month':
      case 'week':
      case 'list':
        this._setView(command)
        return true
      case 'jump':
        this._togglePicker()
        return true
      case 'plot':
        this._setPlotBy(nextPlotBy(this._plotBy))
        return true
      case 'options':
        this._toggleOptions()
        return true
      case 'filter':
        this.renderRoot.querySelector<HTMLInputElement>('input.filter')?.focus()
        return true
      case 'open': {
        if (view !== 'list') {
          this._openDay(this._focus)
          return true
        }
        const row = this._listRows().rows[this._listIndex]
        if (row) this.navigate(this._itemHref(row.item))
        return true
      }
      default:
        break
    }
    if (view === 'list' && command !== 'prev' && command !== 'next') {
      const rows = this._listRows().rows
      const delta = command === 'left' || command === 'up' ? -1 : 1
      this._listIndex = Math.max(0, Math.min(rows.length - 1, this._listIndex + delta))
      this._focusAfterRender = 'list'
      return true
    }
    const next = moveDay(command, view, this._focus)
    if (!next) return false
    this._focus = next
    this._listIndex = 0
    this._popover = null
    this._focusAfterRender = view === 'list' ? 'list' : 'grid'
    return true
  }

  /** The popover's links, for ↑/↓ inside it. */
  private _movePopover(delta: number): void {
    const links = [...this.renderRoot.querySelectorAll<HTMLAnchorElement>('.popover a')]
    const at = links.indexOf(this.shadowRoot?.activeElement as HTMLAnchorElement)
    links[Math.max(0, Math.min(links.length - 1, at + delta))]?.focus()
  }

  private _onKeydown = (e: KeyboardEvent): void => {
    if (!this.open) return
    // Every key stops here, so the page's own shortcuts never see one; only
    // handled keys lose their default action.
    e.stopPropagation()
    if (e.ctrlKey || e.metaKey || e.altKey) return
    const target = e.composedPath()[0] as HTMLElement | undefined
    const tag = target?.tagName
    const handled = (): void => e.preventDefault()
    // The options menu's radios: their arrows, Space and Enter are native,
    // Esc closes the menu, and every other key is an overlay key (`p`, `o`…).
    if (tag === 'INPUT' && (target as HTMLInputElement).type === 'radio') {
      if (e.key === 'Escape') {
        handled()
        this._closeOptions()
        return
      }
      if (e.key.startsWith('Arrow') || e.key === ' ' || e.key === 'Enter' || e.key === 'Tab') return
      const command = overlayCommand(e.key)
      if (command && !MOVES.has(command) && this._run(command)) handled()
      return
    }
    if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA') {
      const toGrid = e.key === 'Escape' || (target!.classList.contains('filter') && (e.key === 'Enter' || e.key === 'ArrowDown'))
      if (!toGrid) return
      if (e.key === 'Escape' && this._picker) this._picker = false
      handled()
      target!.blur()
      this._focusAfterRender = this._view === 'list' ? 'list' : 'grid'
      this.requestUpdate()
      return
    }
    if ((e.key === 'Enter' || e.key === ' ') && (tag === 'BUTTON' || tag === 'A')) return
    if (this._picker && e.key !== 'Escape') return
    if (this._popover && e.key !== 'Escape') {
      const command = overlayCommand(e.key)
      if (command === 'up' || command === 'down') {
        handled()
        this._movePopover(command === 'up' ? -1 : 1)
      }
      return
    }
    const command = overlayCommand(e.key)
    if (command && this._run(command)) handled()
  }

  // ---- render ----

  /** Week / month / quarter / year notes of the period on screen that exist. */
  private _periodLinks(view: CalendarView): TemplateResult[] {
    const { from } = viewRange(view, this._focus, this._weekStart())
    // A week view is about the ISO week of its row's Thursday, and that week
    // belongs to its Thursday's month, quarter and year (Rust's `containing`).
    const keys = periodKeysContaining(view === 'week' ? isoWeekOf(addDays(from, (4 - this._weekStart() + 7) % 7)) : from)
    if (!keys) return []
    const links: [string, string][] = [
      ...(view === 'week' ? ([[keys.week, `Week ${weekNumberLabel(keys.week)}`]] as [string, string][]) : []),
      [keys.month, formatDate(`${keys.month}-01`, this.locale, { month: 'short' })],
      [keys.quarter, `Q${keys.quarter.slice(6)} ${keys.year}`],
      [keys.year, keys.year],
    ]
    return links.flatMap(([key, label]) => {
      const url = this._periodUrl(key)
      return url ? [html`<a href=${url} title=${`Open the ${label} note`}>${label}</a>`] : []
    })
  }

  private _renderPicker(): TemplateResult {
    const months = Array.from({ length: 12 }, (_, i) => `${this._pickerYear}-${String(i + 1).padStart(2, '0')}`)
    const focusMonth = monthOf(this._focus)
    return html`
      <div class="picker" role="dialog" aria-label="Jump to a date">
        <div class="yr">
          <button type="button" class="mbr-icon-btn sm" aria-label="Previous year" @click=${() => this._pickerYear--}>
            ${icon(iconChevronLeft, { size: ICON_SM })}
          </button>
          <strong>${this._pickerYear}</strong>
          <button type="button" class="mbr-icon-btn sm" aria-label="Next year" @click=${() => this._pickerYear++}>
            ${icon(iconChevronRight, { size: ICON_SM })}
          </button>
        </div>
        <div class="months">
          ${months.map(
            (m) => html`
              <button type="button" class=${m === focusMonth ? 'cur' : ''} @click=${() => this._jump(`${m}-01`)}>
                ${formatDate(`${m}-01`, this.locale, { month: 'short' })}
              </button>
            `
          )}
        </div>
        <form @submit=${this._onJumpSubmit}>
          <input
            type="text"
            placeholder="2026-10 or 2026-W44"
            aria-label="Date, month or week"
            aria-invalid=${this._jumpError ? 'true' : 'false'}
            @input=${() => (this._jumpError = false)}
          />
          <button type="submit">Go</button>
        </form>
        ${this._jumpError ? html`<p class="err" role="alert">Try 2026-10, 2026-10-31 or 2026-W44.</p>` : nothing}
      </div>
    `
  }

  private _closePopover = (): void => {
    this._popover = null
    this._focusAfterRender = 'grid'
  }

  private _renderPopover(): TemplateResult | typeof nothing {
    const pop = this._popover
    if (!pop) return nothing
    const day = this._day(pop.date)
    const items = dayItems(day, POPOVER_CAP)
    const { shown, more } = overflowSplit(items.length, POPOVER_CAP, dayItemCount(day) + day.hiddenTasks)
    return html`
      <div class="popover" role="dialog" aria-label=${formatDate(pop.date, this.locale, { dateStyle: 'full' })}>
        <div class="ph">
          <strong>${formatDate(pop.date, this.locale, { weekday: 'long', month: 'long', day: 'numeric' })}</strong>
          <button type="button" class="mbr-icon-btn sm" aria-label="Close" @click=${this._closePopover}>${icon(iconX, { size: ICON_SM })}</button>
        </div>
        <ul>
          ${items.slice(0, shown).map((item) => html`<li>${this._renderItem(item)}</li>`)}
          ${more ? html`<li class="more">+${more} more — filter (/) to narrow</li>` : nothing}
        </ul>
      </div>
    `
  }

  /** A popover / List line: a note (type dot, time, title, folder) or a task (checkbox, time, text, why, note). */
  private _renderItem(item: DayItem, attrs?: { i: number; focus: boolean }): TemplateResult {
    let body: TemplateResult
    let title: string | typeof nothing = nothing
    if (item.kind === 'note') {
      const e = item.entry
      const dot = e.color === null ? '' : `background:var(--mbr-cal-type-${e.color})`
      const time = e.minutes !== null ? html`<span class="time">${formatClock(e.minutes, this.locale)}</span>` : nothing
      const folder = e.folder ? html`<span class="folder">${e.folder}</span>` : nothing
      body = html`<i class="dot" style=${dot}></i>${time}<span class="t">${e.daily ? 'Daily note' : e.title}</span>${folder}`
    } else {
      const hit = item.hit
      const minutes = taskMinutes(hit.time)
      const time = minutes !== null ? html`<span class="time">${formatClock(minutes, this.locale)}</span>` : nothing
      const text = taskDisplayText(hit.text)
      title = `${text} — ${hit.note}`
      body = html`<i class=${`check ${hit.status}`} aria-hidden="true"></i>${time}<span class="t">${text}</span
        ><span class="placed">${PLACED_LABELS[hit.placed_by]}</span><span class="folder">${hit.note}</span>`
    }
    return html`<a
      class=${`${attrs ? 'row' : 'prow'}${item.kind === 'task' ? ` task ${item.hit.status}` : ''}`}
      href=${this._itemHref(item)}
      title=${title}
      data-i=${attrs?.i ?? nothing}
      tabindex=${attrs ? (attrs.focus ? 0 : -1) : nothing}
    >${body}</a>`
  }

  private _onListFocus = (e: Event): void => {
    const i = (e.target as HTMLElement).dataset?.i
    if (i !== undefined) this._listIndex = Number(i)
  }

  private _renderList(): TemplateResult {
    const { rows, more } = this._listRows()
    const out: (TemplateResult | typeof nothing)[] = []
    rows.forEach((row, i) => {
      if (i === 0 || rows[i - 1].date !== row.date) {
        const label = formatDate(row.date, this.locale, { weekday: 'long', month: 'long', day: 'numeric' })
        out.push(html`<h3 class=${row.date === this._today ? 'today' : ''}><time datetime=${row.date}>${label}</time></h3>`)
      }
      out.push(this._renderItem(row.item, { i, focus: i === this._listIndex }))
      const left = more.get(row.date)
      if (left && rows[i + 1]?.date !== row.date) out.push(html`<p class="more">+${left} more — filter (/) to narrow</p>`)
    })
    return html`<div class="list" @focusin=${this._onListFocus}>${out}</div>`
  }

  private _renderView(view: CalendarView): TemplateResult {
    const fns = this._callbacks()
    const weekNumbers = this.config.weekNumbers ?? (this._index ? this._index.week.length > 0 : false)
    if (view === 'list') return this._renderList()
    if (view === 'week') {
      return html`
        <mbr-calendar-week-grid
          .start=${viewRange('week', this._focus, this._weekStart()).from}
          .focused=${this._focus}
          .today=${this._today}
          .locale=${this.locale}
          .dayEntries=${fns.week}
          .dayCount=${fns.count}
          .dayMarks=${fns.marks}
          @mbr-calendar-focus=${this._onGridFocus}
          @mbr-calendar-day=${this._onGridDay}
        ></mbr-calendar-week-grid>
      `
    }
    return html`
      <mbr-calendar-month-grid
        .month=${monthOf(this._focus)}
        .focused=${this._focus}
        .today=${this._today}
        .weekStart=${this._weekStart()}
        .weekNumbers=${weekNumbers}
        .locale=${this.locale}
        .dayEntries=${fns.month}
        .dayCount=${fns.count}
        .dayMarks=${fns.marks}
        .weekHref=${this._weekHref}
        @mbr-calendar-focus=${this._onGridFocus}
        @mbr-calendar-day=${this._onGridDay}
      ></mbr-calendar-month-grid>
    `
  }

  /**
   * What narrows the view, at a glance: an explicit scope (its ✕ widens to
   * all notes), "This folder" (its ✕ turns it off) and a plot-by other than
   * note date (its ✕ goes back to it). Nothing while all notes show by note date.
   */
  private _renderChips(): TemplateResult[] {
    const chip = (label: string, clear: string, onClear: () => void) => html`
      <span class="chip">${label}<button type="button" class="mbr-icon-btn sm" aria-label=${clear} title=${clear} @click=${onClear}>
          ${icon(iconX, { size: ICON_SM })}
        </button></span
      >
    `
    const chips: TemplateResult[] = []
    if (this._scoped) chips.push(chip(this._label, 'Show all notes', () => this.widen()))
    else if (this._folderOnly && this._folder) {
      chips.push(chip(`Folder: ${this._folder}`, 'Show all folders', () => this._setFolderOnly(false)))
    }
    if (this._plotBy !== 'date') {
      chips.push(chip(`Plot by ${PLOT_BY_LABELS[this._plotBy].toLowerCase()}`, 'Plot by note date', () => this._setPlotBy('date')))
    }
    return chips
  }

  /** One radio group of the options menu. */
  private _renderRadios<T extends string>(
    legend: string,
    name: string,
    options: readonly T[],
    label: (value: T) => string,
    current: T,
    choose: (value: T) => void
  ): TemplateResult {
    return html`
      <fieldset>
        <legend>${legend}</legend>
        ${options.map(
          (value) => html`
            <label>
              <input type="radio" name=${name} value=${value} .checked=${value === current} @change=${() => choose(value)} />
              <span>${label(value)}</span>
            </label>
          `
        )}
      </fieldset>
    `
  }

  /**
   * The options menu: scope ("This folder", only when opened from a page and
   * not over an explicit scope), plot-by, and tasks (only with a task source).
   */
  private _renderOptions(): TemplateResult {
    const folder = this._folder && !this._scoped ? this._folder : ''
    return html`
      <div class="options" role="group" aria-label="Calendar options">
        ${folder
          ? this._renderRadios(
              'Scope',
              'scope',
              ['all', 'folder'] as const,
              (v) => (v === 'all' ? 'All notes' : `This folder: ${folder}`),
              this._folderOnly ? 'folder' : 'all',
              (v) => this._setFolderOnly(v === 'folder')
            )
          : nothing}
        ${this._renderRadios('Plot by', 'plot', PLOT_BYS, (p) => PLOT_BY_LABELS[p], this._plotBy, (p) => this._setPlotBy(p))}
        ${this.taskSource
          ? this._renderRadios('Tasks', 'tasks', TASKS_SHOWS, (t) => TASKS_SHOW_LABELS[t], this._tasksShow, (t) => this._setTasksShow(t))
          : nothing}
      </div>
    `
  }

  override render(): TemplateResult | typeof nothing {
    if (!this.open) return nothing
    const view = this._view ?? 'month'
    const buckets = this._bucketsNow()
    const { from, to } = viewRange(view, this._focus, this._weekStart())
    const { shown, total, tasksShown, taskTotal } = this._countRange(from, to)
    const where = view === 'week' ? 'this week' : `in ${formatDate(from, this.locale, { month: 'long' })}`
    const of = (n: number) => (this._terms.length ? `${n} of ` : '')
    const tasksText = this._tasksOn && this._tasks ? ` · ${of(tasksShown)}${taskTotal} ${taskTotal === 1 ? 'task' : 'tasks'}` : ''
    const count = `${of(shown)}${total} ${total === 1 ? 'note' : 'notes'}${tasksText} ${where}`
    const nothingShown = buckets !== null && shown === 0 && tasksShown === 0
    const before = nothingShown ? this._nearMatch(-1, from, to) : null
    const after = nothingShown ? this._nearMatch(1, from, to) : null
    const unit = view === 'week' ? 'week' : 'month'
    const step = (d: number) => () => this._run(d < 0 ? 'prev' : 'next')
    const viewName = (v: CalendarView): string => (v === 'list' ? 'List' : v === 'week' ? 'Week' : 'Month')
    const nav = html`
      <div class="nav">
        <button type="button" class="mbr-icon-btn" aria-label=${`Previous ${unit}`} title=${`Previous ${unit} ([)`} @click=${step(-1)}>
          ${icon(iconChevronLeft)}
        </button>
        <button type="button" class="mbr-icon-btn labeled" title="Today (t)" @click=${() => this._run('today')}>Today</button>
        <button type="button" class="mbr-icon-btn" aria-label=${`Next ${unit}`} title=${`Next ${unit} (])`} @click=${step(1)}>
          ${icon(iconChevronRight)}
        </button>
        <button
          type="button"
          class="title mbr-icon-btn labeled"
          aria-expanded=${this._picker ? 'true' : 'false'}
          title="Jump to… (g)"
          @click=${() => this._togglePicker()}
        >
          ${viewTitle(view, this._focus, this._weekStart(), this.locale)}${icon(iconChevronDown, { size: ICON_SM })}
        </button>
        <span class="links">${this._periodLinks(view)}</span>
      </div>
    `
    const bar = html`
      <div class="bar">
        ${nav}
        <div class="views" role="group" aria-label="View">
          ${(['month', 'week', 'list'] as const).map(
            (v) => html`
              <button
                type="button"
                aria-pressed=${v === view ? 'true' : 'false'}
                title=${`${viewName(v)} (${v === 'list' ? 'a' : v[0]})`}
                @click=${() => this._setView(v)}
              >
                ${viewName(v)}
              </button>
            `
          )}
        </div>
        <button type="button" class="mbr-icon-btn" aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)" @click=${() => (this._help = !this._help)}>
          ${icon(iconCircleHelp)}
        </button>
        <button type="button" class="mbr-icon-btn" aria-label="Close" title="Close (Esc)" @click=${() => this._close()}>${icon(iconX)}</button>
      </div>
    `
    const changed = this._optionsChanged()
    const tools = html`
      <div class="tools">
        ${this._renderChips()}
        <input
          class="filter"
          type="search"
          placeholder="Filter (/)"
          aria-label="Filter notes"
          .value=${this._filterText}
          @input=${(e: Event) => this._setFilter((e.target as HTMLInputElement).value)}
          autocomplete="off"
          spellcheck="false"
        />
        <button
          type="button"
          class="mbr-icon-btn options-btn"
          aria-label=${changed ? 'Calendar options (changed)' : 'Calendar options'}
          aria-haspopup="true"
          aria-expanded=${this._options ? 'true' : 'false'}
          title="Options: scope, plot by, tasks (o)"
          @click=${() => this._toggleOptions()}
        >
          ${icon(iconSlidersHorizontal)}${changed ? html`<span class="badge" aria-hidden="true"></span>` : nothing}
        </button>
        <span class="count" role="status">${buckets ? count : 'Loading…'}</span>
      </div>
    `
    const empty = nothingShown
        ? html`
            <div class="empty">
              ${this._terms.length ? 'No matches' : 'No notes'} ${where}.
              ${before
                ? html`<button type="button" class="linkish" @click=${() => this._setFocus(before)}>‹ Previous ${unit} with matches</button>`
                : nothing}
              ${after
                ? html`<button type="button" class="linkish" @click=${() => this._setFocus(after)}>Next ${unit} with matches ›</button>`
                : nothing}
            </div>
          `
        : nothing
    const help = this._help
      ? html`
          <div class="help" role="dialog" aria-label="Calendar keys">
            <table>
              ${OVERLAY_SHORTCUTS.map((sc) => html`<tr><td><kbd>${sc.keys}</kbd></td><td>${sc.description}</td></tr>`)}
            </table>
          </div>
        `
      : nothing
    return html`
      <div class="backdrop" @click=${() => this._close()}></div>
      <div class="dialog" role="dialog" aria-modal="true" aria-label="Calendar" @pointerdown=${this._onDialogPointer}>
        ${bar} ${tools} ${empty}
        <div class=${`view ${view}`}>${buckets ? this._renderView(view) : nothing}</div>
        ${this._picker ? this._renderPicker() : nothing}${this._options ? this._renderOptions() : nothing}${this._renderPopover()}${help}
      </div>
    `
  }

  static override styles = [
    ICON_BUTTON_CSS,
    KBD_CSS,
    css`
      :host {
        display: contents;
        --mbr-kbd-size: 0.75rem;
      }
      .backdrop {
        position: fixed;
        inset: 0;
        z-index: 1000;
        background: rgba(0, 0, 0, 0.4);
      }
      .dialog {
        position: fixed;
        inset: 2vh 2vw;
        z-index: 1001;
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        box-sizing: border-box;
        padding: 0.6rem 0.8rem 0.8rem;
        border-radius: 10px;
        background: var(--pico-background-color, #fff);
        color: var(--pico-color);
        box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
        font-size: 0.9rem;
      }
      .bar,
      .tools,
      .nav {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.3rem;
        min-width: 0;
      }
      .nav {
        flex: 1;
      }
      .title {
        font-weight: 700;
        font-size: 1.05rem;
        width: auto;
        padding-inline: 0.4rem;
        gap: 0.25rem;
      }
      .labeled {
        width: auto;
        padding-inline: 0.5rem;
      }
      .links {
        display: flex;
        gap: 0.6rem;
        font-size: 0.8rem;
      }
      a {
        color: var(--pico-primary);
        text-decoration: none;
      }
      a:hover {
        text-decoration: underline;
      }
      .views {
        display: inline-flex;
        border: 1px solid var(--pico-muted-border-color, #ccc);
        border-radius: 8px;
        overflow: hidden;
      }
      .views button,
      .months button,
      form button,
      .chip,
      .linkish {
        margin: 0;
        padding: 0.25rem 0.6rem;
        border: 0;
        background: none;
        color: var(--pico-color);
        font: inherit;
        font-size: 0.85rem;
        cursor: pointer;
      }
      .views button[aria-pressed='true'] {
        background: var(--pico-primary);
        color: var(--pico-primary-inverse, #fff);
      }
      .chip {
        display: inline-flex;
        align-items: center;
        gap: 0.2rem;
        padding: 0.1rem 0.5rem;
        border-radius: 999px;
        max-width: 18rem;
        padding-right: 0.1rem;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        background: var(--mbr-cal-daily-bg, rgba(1, 114, 173, 0.12));
        cursor: default;
      }
      .chip .mbr-icon-btn.sm {
        width: 22px;
        height: 22px;
      }
      .filter {
        flex: 1 1 14rem;
        min-width: 6rem;
        max-width: 36rem;
        height: 2rem;
        margin: 0;
        padding: 0 0.5rem;
        font-size: 0.9rem;
      }
      .options-btn {
        position: relative;
      }
      .badge {
        position: absolute;
        top: 5px;
        right: 5px;
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--pico-primary);
      }
      .options {
        width: 15rem;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .options fieldset {
        margin: 0;
        padding: 0;
        border: 0;
      }
      .options legend {
        margin-bottom: 0.2rem;
        padding: 0;
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.05em;
        text-transform: uppercase;
        color: var(--pico-muted-color, #666);
      }
      .options label {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        margin: 0;
        padding: 0.15rem 0;
        font-size: 0.85rem;
        cursor: pointer;
      }
      .options input[type='radio'] {
        flex: none;
        width: 1rem;
        height: 1rem;
        margin: 0;
      }
      .options label span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .check {
        flex: none;
        width: 9px;
        height: 9px;
        border: 1.5px solid currentColor;
        border-radius: 2px;
      }
      .check.done {
        background: currentColor;
      }
      .task.done .t {
        text-decoration: line-through;
        color: var(--pico-muted-color);
      }
      .placed {
        flex: none;
        font-size: 0.7rem;
        color: var(--pico-muted-color);
        text-transform: uppercase;
        letter-spacing: 0.04em;
      }
      .count {
        margin-left: auto;
        font-size: 0.8rem;
        color: var(--pico-muted-color);
      }
      .empty {
        font-size: 0.85rem;
        color: var(--pico-muted-color);
      }
      .linkish {
        padding: 0;
        color: var(--pico-primary);
      }
      .view {
        flex: 1;
        min-height: 0;
        overflow: auto;
      }
      .view.month,
      .view.week {
        overflow: hidden;
      }
      .list h3 {
        position: sticky;
        top: 0;
        z-index: 1;
        margin: 0.6rem 0 0.2rem;
        padding: 0.2rem 0;
        background: var(--pico-background-color, #fff);
        font-size: 0.85rem;
      }
      .list h3.today {
        color: var(--pico-primary);
      }
      .row,
      .prow {
        display: flex;
        align-items: center;
        gap: 0.45rem;
        padding: 0.3rem 0.4rem;
        border-radius: 6px;
        color: var(--pico-color);
        outline: none;
      }
      .row:hover,
      .row:focus,
      .prow:hover,
      .prow:focus {
        background: var(--mbr-control-hover-bg, rgba(0, 0, 0, 0.06));
        text-decoration: none;
      }
      .row:focus-visible,
      .prow:focus-visible {
        box-shadow: inset 0 0 0 2px var(--pico-primary);
      }
      .dot {
        flex: none;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--pico-muted-color, #888);
      }
      .t {
        min-width: 0;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
      }
      .time,
      .folder {
        flex: none;
        font-size: 0.8rem;
        color: var(--pico-muted-color);
        font-variant-numeric: tabular-nums;
      }
      .folder {
        margin-left: auto;
        max-width: 40%;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .picker,
      .popover,
      .options,
      .help {
        position: fixed;
        z-index: 1002;
        box-sizing: border-box;
        max-width: calc(100vw - 16px);
        max-height: 70vh;
        overflow: auto;
        padding: 0.6rem;
        border: 1px solid var(--pico-muted-border-color, #ccc);
        border-radius: 8px;
        background: var(--pico-background-color, #fff);
        box-shadow: 0 6px 24px rgba(0, 0, 0, 0.25);
      }
      .picker {
        top: calc(2vh + 3.2rem);
        left: calc(2vw + 4rem);
        width: 17rem;
      }
      .help {
        top: calc(2vh + 3.2rem);
        right: calc(2vw + 1rem);
        font-size: 0.85rem;
      }
      .help td {
        padding: 0.15rem 0.4rem;
        border: 0;
      }
      .popover {
        width: 20rem;
      }
      .popover ul {
        list-style: none;
        margin: 0;
        padding: 0;
      }
      .popover li {
        margin: 0;
        list-style: none;
      }
      .popover .more,
      .list .more {
        margin: 0.2rem 0.4rem;
        font-size: 0.8rem;
        color: var(--pico-muted-color);
      }
      .ph,
      .yr {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 0.4rem;
      }
      .months {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 0.2rem;
        margin-bottom: 0.5rem;
      }
      .months button {
        border-radius: 6px;
        text-align: center;
      }
      .months button:hover {
        background: var(--mbr-control-hover-bg, rgba(0, 0, 0, 0.06));
      }
      .months button.cur {
        background: var(--pico-primary);
        color: var(--pico-primary-inverse, #fff);
      }
      form {
        display: flex;
        gap: 0.3rem;
        margin: 0;
      }
      form input {
        flex: 1;
        height: 2rem;
        margin: 0;
        padding: 0 0.4rem;
        font-size: 0.85rem;
      }
      form button {
        border: 1px solid var(--pico-muted-border-color, #ccc);
        border-radius: 6px;
      }
      .err {
        margin: 0.3rem 0 0;
        font-size: 0.8rem;
        color: var(--mbr-cal-overdue, #c62828);
      }
      @media (max-width: 600px) {
        .dialog {
          inset: 0;
          border-radius: 0;
          padding: calc(0.4rem + env(safe-area-inset-top, 0px)) calc(0.5rem + env(safe-area-inset-right, 0px))
            calc(0.5rem + env(safe-area-inset-bottom, 0px)) calc(0.5rem + env(safe-area-inset-left, 0px));
        }
        .nav {
          flex-basis: 100%;
        }
        .links {
          font-size: 0.8rem;
        }
        .options-btn {
          width: 44px;
          height: 44px;
        }
        .options label {
          min-height: 44px;
        }
        .filter {
          font-size: 16px;
          max-width: none;
        }
        .count {
          margin-left: 0;
          width: 100%;
        }
        .picker {
          left: 8px;
        }
        .view.week {
          overflow: auto;
        }
      }
    `,
  ]
}
