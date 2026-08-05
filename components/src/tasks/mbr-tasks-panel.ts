import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, property, query, state } from 'lit/decorators.js'
import { safeHref } from '../safe-href.js'
import { renderTaskCard } from './task-card.js'
import {
  buildDisplayGroups,
  buildRows,
  groupRowIndex,
  taskAt,
  taskHref,
  type DisplayGroup,
  type TaskRow,
} from './task-groups.js'
import { buildTaskFolderTree, folderScopeValue, type FolderTreeNode } from './folder-tree.js'
import { formatDateHeading, progressPercent } from './task-format.js'
import type {
  DueFilter,
  TaskHit,
  TaskMode,
  TaskPriority,
  TaskQueryRequest,
  TaskQueryResponse,
  TaskStatus,
  TaskToggler,
} from './types.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-tasks-panel': MbrTasksPanelElement
  }
}

/** Debounce before a keystroke in the filter field becomes a request. */
const FILTER_DEBOUNCE_MS = 150

/** Cap on returned tasks; mirrors `task_query::DEFAULT_TASK_LIMIT`. */
const TASK_LIMIT = 500

/** Filter-popover option lists, in display order. */
const STATUS_OPTIONS: ReadonlyArray<{ value: TaskStatus; label: string }> = [
  { value: 'open', label: 'Incomplete' },
  { value: 'done', label: 'Complete' },
  { value: 'canceled', label: 'Canceled' },
]

const PRIORITY_OPTIONS: ReadonlyArray<{ value: TaskPriority; label: string }> = [
  { value: 'normal', label: 'Normal' },
  { value: 'high', label: 'High' },
  { value: 'urgent', label: 'Urgent' },
]

/**
 * The true target of a keydown, seeing through shadow-root retargeting.
 *
 * The panel listens on `document`, where an event from its own shadow root is
 * retargeted to the host — so `e.target` is always `<mbr-tasks-panel>` and says
 * nothing about which control the user is on. Same reason `isInputTarget` in
 * `mbr-keys.ts` uses `composedPath`.
 */
function realTarget(e: KeyboardEvent): HTMLElement | null {
  const target = e.composedPath()[0]
  return target instanceof HTMLElement ? target : null
}

/** Tag name of the real target, uppercased, or `''`. */
function targetTag(e: KeyboardEvent): string {
  return realTarget(e)?.tagName ?? ''
}

/** Whether a focused control activates itself on Enter and must keep the key. */
function ownsEnter(target: HTMLElement | null): boolean {
  if (!target) return false
  const tag = target.tagName
  if (tag === 'BUTTON' || tag === 'SELECT') return true
  return tag === 'INPUT' && (target as HTMLInputElement).type === 'checkbox'
}

/**
 * Identity of one task across a re-query.
 *
 * Path + line, not the array index: a toggle re-queries, and with the default
 * incomplete-only filter the task it completed leaves the list — so index `3`
 * afterwards is a different task than index `3` before.
 */
function taskKey(hit: TaskHit): string {
  return `${hit.path}:${hit.line}`
}

const DUE_OPTIONS: ReadonlyArray<{ value: DueFilter; label: string }> = [
  { value: 'any', label: 'Any due date' },
  { value: 'overdue', label: 'Overdue' },
  { value: 'today', label: 'Due today' },
  { value: 'tomorrow', label: 'Due tomorrow' },
  { value: 'upcoming', label: 'Upcoming' },
  { value: 'none', label: 'No due date' },
]

/**
 * `<mbr-tasks-panel>` — the two-pane task browser.
 *
 * Lives in the lazy `mbr-tasks.min.js` chunk and imports nothing stateful from
 * the main bundle: the endpoint and the URL resolver arrive as properties from
 * the `<mbr-tasks>` trigger, exactly as `<mbr-mini-graph>` receives its services
 * from `<mbr-info>`.
 *
 * # Filtering is the server's job
 *
 * Every filter change is a new `POST /.mbr/tasks`. Nothing is filtered client
 * side, because the two modes count very differently — category totals cover
 * every task in the file including ones the filter hid, calendar totals cover
 * everything except the status filter — and having one authoritative
 * implementation of that is the whole reason grouping lives on the server.
 *
 * # Keyboard model
 *
 * Focus walks a flat row sequence of headings and tasks (see `task-groups.ts`);
 * a collapsed group contributes its heading but none of its tasks, so `↓`/`↑`
 * skip past it. `←` collapses the focused row's group and parks focus on its
 * heading, so `→` can expand it again — a heading-less model would lose the
 * group the moment it collapsed.
 */
@customElement('mbr-tasks-panel')
export class MbrTasksPanelElement extends LitElement {
  /** Query endpoint, injected by the trigger. */
  @property({ attribute: false })
  endpoint = '/.mbr/tasks'

  /** Root-relative URL resolver, injected by the trigger (`shared.resolveUrl`). */
  @property({ attribute: false })
  resolveHref: (path: string) => string = (path) => path

  /**
   * Whether task status can be written. Injected rather than read from
   * `shared.isEditEnabled()` because that module is main-bundle state; this is
   * the same seam `resolveHref` uses.
   *
   * When false the checkboxes stay `disabled` and `Space`/`x` stay unbound —
   * no control is shown that cannot work.
   */
  @property({ attribute: false })
  editEnabled = false

  /** Writes one task's status; injected by the trigger. See `task-toggle.ts`. */
  @property({ attribute: false })
  toggleTask: TaskToggler | null = null

  /**
   * Today, for overdue marking and date headings. A property so tests can pin
   * it; the server does its own bucketing against its own clock.
   */
  @property({ attribute: false })
  today: Date = new Date()

  /** Locale for date formatting; tests pin it to keep assertions stable. */
  @property({ attribute: false })
  locale: string | undefined = undefined

  // === Query state ===
  @state() private _q = ''
  @state() private _folder: string | null = null
  @state() private _statuses: TaskStatus[] = ['open']
  @state() private _priorities: TaskPriority[] = []
  @state() private _due: DueFilter = 'any'
  @state() private _mode: TaskMode = 'category'

  // === Response state ===
  @state() private _response: TaskQueryResponse | null = null
  @state() private _folderTree: FolderTreeNode | null = null
  @state() private _loading = false
  @state() private _error: string | null = null

  // === UI state ===
  @state() private _collapsed = new Set<string>()
  @state() private _expandedFolders = new Set<string>(['/'])
  @state() private _filtersOpen = false
  @state() private _focusRow = -1
  /** 0 = folder pane, 1 = results pane. Drives `Tab` and the Ctrl scroll keys. */
  @state() private _activePaneIndex = 1

  /**
   * Optimistic status overrides, keyed by {@link taskKey}.
   *
   * An entry lives from the click until the re-query that followed the write
   * has landed — not until the write returns — so the card never flickers back
   * to the old status in the gap between the two.
   */
  @state() private _pending: ReadonlyMap<string, TaskStatus> = new Map()

  /** A failed write's message, shown above the results. */
  @state() private _notice: string | null = null

  /** Keys with a write in flight, so a double press cannot race itself. */
  private _inFlight = new Set<string>()

  @query('#tasks-filter')
  private _input!: HTMLInputElement

  private _debounceTimeout: number | null = null
  private _abortController: AbortController | null = null

  /**
   * Monotonic id for query runs. Bumped when a query starts and when the panel
   * disconnects, so a slow in-flight request can detect that it has been
   * superseded and skip its state writes. The debounce only prevents *starting*
   * a request per keystroke; a request slower than that can still overlap the
   * next one and land last, leaving results that do not match the filters on
   * screen.
   */
  private _generation = 0

  override connectedCallback() {
    super.connectedCallback()
    document.addEventListener('keydown', this._handleKeydown)
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._handleKeydown)
    // Invalidate anything in flight: the element is destroyed on close and a
    // late response must not write to a detached element.
    this._generation++
    if (this._debounceTimeout !== null) {
      clearTimeout(this._debounceTimeout)
      this._debounceTimeout = null
    }
    this._abortController?.abort()
    this._abortController = null
  }

  override firstUpdated() {
    this._input?.focus()
    void this._runQuery()
  }

  // ========================================
  // Querying
  // ========================================

  /** The exact body posted to `/.mbr/tasks` for the current filter state. */
  public requestBody(): TaskQueryRequest {
    return {
      q: this._q,
      folder: folderScopeValue(this._folder),
      statuses: [...this._statuses],
      priorities: [...this._priorities],
      due: this._due,
      mode: this._mode,
      limit: TASK_LIMIT,
    }
  }

  private _scheduleQuery() {
    if (this._debounceTimeout !== null) {
      clearTimeout(this._debounceTimeout)
    }
    this._debounceTimeout = window.setTimeout(() => {
      this._debounceTimeout = null
      void this._runQuery()
    }, FILTER_DEBOUNCE_MS)
  }

  /**
   * Run a query.
   *
   * `keepFocus` is set by the refresh that follows a toggle: a filter change is
   * a new list and deserves a reset, but a toggle is the *same* list one status
   * later, and dropping focus there would make the next `Space` type into the
   * filter field instead of toggling the next task.
   */
  private async _runQuery(options: { keepFocus?: boolean } = {}): Promise<void> {
    this._abortController?.abort()
    const abortController = new AbortController()
    this._abortController = abortController

    const generation = ++this._generation
    const isStale = () => generation !== this._generation
    this._loading = true
    this._error = null

    try {
      const response = await fetch(this.endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(this.requestBody()),
        signal: abortController.signal,
      })
      if (!response.ok) {
        throw new Error(
          response.status === 404
            ? 'The task browser is disabled on this server.'
            : `Task query failed: ${response.status}`
        )
      }
      const data = (await response.json()) as TaskQueryResponse
      // Every state write happens after the last await, so a superseded run
      // cannot leave the folder tree describing one query and the groups another.
      if (isStale()) return
      const focusedKey = options.keepFocus ? this._focusedTaskKey() : null
      const previousRow = this._focusRow
      this._response = data
      this._folderTree = buildTaskFolderTree(data.folders ?? [])
      this._focusRow = options.keepFocus ? this._rowFor(focusedKey, previousRow) : -1
    } catch (err) {
      if (err instanceof Error && err.name === 'AbortError') return
      if (isStale()) return
      console.error('Task query error:', err)
      this._error = err instanceof Error ? err.message : 'Task query failed'
      this._response = null
    } finally {
      // Only the newest run owns the loading indicator; a superseded run
      // clearing it would hide the spinner while a request is still running.
      if (generation === this._generation) {
        this._loading = false
      }
    }
  }

  // ========================================
  // Derived view model
  // ========================================

  /**
   * Memo for the derived view model.
   *
   * `render()` and every keyboard handler read `_groups`/`_rows`, so without
   * this a single keypress rebuilds the whole projection half a dozen times.
   * A `limit` of 500 tasks makes that cheap either way, but the identity
   * stability also matters: `_rows` is compared by index across a collapse.
   */
  private _viewCache: {
    response: TaskQueryResponse | null
    mode: TaskMode
    collapsed: ReadonlySet<string>
    groups: DisplayGroup[]
    rows: TaskRow[]
  } | null = null

  private get _view(): { groups: DisplayGroup[]; rows: TaskRow[] } {
    const cache = this._viewCache
    if (
      cache &&
      cache.response === this._response &&
      cache.mode === this._mode &&
      cache.collapsed === this._collapsed
    ) {
      return cache
    }
    // Every input is replaced rather than mutated (`_collapsed` is a fresh Set
    // per toggle), so identity comparison is a sound cache key.
    const groups = buildDisplayGroups(this._response, this._mode)
    const rows = buildRows(groups, this._collapsed)
    this._viewCache = {
      response: this._response,
      mode: this._mode,
      collapsed: this._collapsed,
      groups,
      rows,
    }
    return this._viewCache
  }

  private get _groups(): DisplayGroup[] {
    return this._view.groups
  }

  private get _rows(): TaskRow[] {
    return this._view.rows
  }

  // ========================================
  // Filter handlers
  // ========================================

  private _handleFilterInput(e: Event) {
    this._q = (e.target as HTMLInputElement).value
    this._focusRow = -1
    this._scheduleQuery()
  }

  private _selectFolder(path: string | null) {
    this._folder = path
    this._focusRow = -1
    void this._runQuery()
  }

  private _setMode(mode: TaskMode) {
    if (this._mode === mode) return
    this._mode = mode
    // Group keys differ between modes, so a stale collapse set would silently
    // hide the wrong headings.
    this._collapsed = new Set()
    this._focusRow = -1
    void this._runQuery()
  }

  /**
   * Toggle one status checkbox.
   *
   * An empty selection is never sent: the server reads `[]` as "incomplete
   * only" (see `Filters::new`), so clearing every box would quietly show open
   * tasks with nothing checked. Unchecking the last one is refused instead.
   *
   * The refusal has to put the checkbox back by hand. Lit dirty-checks a
   * binding against the value it last committed, so re-rendering with an
   * unchanged `_statuses` writes nothing — and the DOM would keep the
   * user's unchecked box while the model still said "open".
   */
  private _toggleStatus(value: TaskStatus, input: HTMLInputElement) {
    const next = this._statuses.includes(value)
      ? this._statuses.filter((s) => s !== value)
      : [...this._statuses, value]
    if (next.length === 0) {
      input.checked = true
      return
    }
    this._statuses = next
    void this._runQuery()
  }

  /** Toggle one priority checkbox. Empty means "all", which is a valid state. */
  private _togglePriority(value: TaskPriority) {
    this._priorities = this._priorities.includes(value)
      ? this._priorities.filter((p) => p !== value)
      : [...this._priorities, value]
    void this._runQuery()
  }

  private _setDue(value: DueFilter) {
    this._due = value
    void this._runQuery()
  }

  // ========================================
  // Toggling
  // ========================================

  /** {@link taskKey} of the focused row, or `null` when it is not a task. */
  private _focusedTaskKey(): string | null {
    const hit = taskAt(this._groups, this._rows[this._focusRow])
    return hit ? taskKey(hit) : null
  }

  /**
   * Row index for `key` after a refresh, falling back to `previousRow` clamped
   * into range — which is what happens when a completed task drops out of an
   * incomplete-only view, and leaves focus on whatever took its place.
   */
  private _rowFor(key: string | null, previousRow: number): number {
    const rows = this._rows
    if (rows.length === 0) return -1
    if (key !== null) {
      const found = rows.findIndex((row) => {
        const hit = taskAt(this._groups, row)
        return hit !== null && taskKey(hit) === key
      })
      if (found >= 0) return found
    }
    return Math.min(previousRow, rows.length - 1)
  }

  /** The status to draw for a task: its optimistic override, else the server's. */
  private _statusOf(hit: TaskHit): TaskStatus {
    return this._pending.get(taskKey(hit)) ?? hit.status
  }

  private _setPending(key: string, status: TaskStatus | null) {
    const next = new Map(this._pending)
    if (status === null) {
      next.delete(key)
    } else {
      next.set(key, status)
    }
    this._pending = next
  }

  /**
   * Write one task's status, optimistically.
   *
   * The card flips immediately, the write goes out, and on success the whole
   * query is re-run so the group's `x/y` and progress bar catch up — the server
   * invalidates its task index inside the same request, so the refresh is
   * accurate the moment the write returns. On failure the flip is reverted and
   * the reason is shown; a `409` also refreshes, because a conflict means the
   * view is describing a file that has since changed.
   */
  private async _writeStatus(hit: TaskHit, to: TaskStatus): Promise<void> {
    const toggle = this.toggleTask
    if (!this.editEnabled || !toggle) return
    const key = taskKey(hit)
    if (this._inFlight.has(key)) return

    this._inFlight.add(key)
    this._setPending(key, to)
    try {
      const outcome = await toggle({ path: hit.path, line: hit.line, to })
      if (outcome.ok) {
        this._notice = null
        // The override outlives the write and is dropped only once the fresh
        // response is on screen, so the card never shows the old status again.
        await this._runQuery({ keepFocus: true })
        return
      }
      this._notice = outcome.message
      if (outcome.kind === 'conflict') {
        await this._runQuery({ keepFocus: true })
      }
    } finally {
      this._inFlight.delete(key)
      this._setPending(key, null)
    }
  }

  /** Left click / `Space`: done ↔ open. */
  private _toggleDone(hit: TaskHit) {
    void this._writeStatus(hit, this._statusOf(hit) === 'done' ? 'open' : 'done')
  }

  /** Right click / `x`: canceled ↔ open. */
  private _toggleCanceled(hit: TaskHit) {
    void this._writeStatus(hit, this._statusOf(hit) === 'canceled' ? 'open' : 'canceled')
  }

  // ========================================
  // Collapse / expand
  // ========================================

  private _toggleCollapse(key: string) {
    const next = new Set(this._collapsed)
    if (next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    this._collapsed = next
  }

  private _toggleFolderExpansion(path: string) {
    const next = new Set(this._expandedFolders)
    if (next.has(path)) {
      next.delete(path)
    } else {
      next.add(path)
    }
    this._expandedFolders = next
  }

  // ========================================
  // Keyboard
  // ========================================

  private _close() {
    this.dispatchEvent(new CustomEvent('mbr-tasks-close'))
  }

  private _handleKeydown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      if (this._filtersOpen) {
        this._filtersOpen = false
        return
      }
      this._close()
      return
    }

    if (e.key === 'Tab') {
      e.preventDefault()
      this._activePaneIndex = this._activePaneIndex === 0 ? 1 : 0
      return
    }

    if (e.ctrlKey && !e.metaKey) {
      switch (e.key.toLowerCase()) {
        case 'n':
          e.preventDefault()
          this._moveFocus(1)
          return
        case 'p':
          e.preventDefault()
          this._moveFocus(-1)
          return
        case 'd':
          e.preventDefault()
          this._scrollActivePane(0.5)
          return
        case 'u':
          e.preventDefault()
          this._scrollActivePane(-0.5)
          return
        case 'f':
          e.preventDefault()
          this._scrollActivePane(1)
          return
        case 'b':
          e.preventDefault()
          this._scrollActivePane(-1)
          return
      }
      return
    }

    if (e.metaKey || e.altKey) return

    // A <select> owns its own arrow keys (the due-range filter), and buttons,
    // selects and checkboxes own Enter. Hijacking those would leave the filter
    // popover and the headings unusable once they have DOM focus. The filter
    // TEXT field is deliberately not exempt: Enter there opens the focused
    // task, which is the whole point of typing a filter.
    if (targetTag(e) === 'SELECT' && e.key.startsWith('Arrow')) return
    if (e.key === 'Enter' && ownsEnter(realTarget(e))) return

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        this._moveFocus(1)
        break
      case 'ArrowUp':
        e.preventDefault()
        this._moveFocus(-1)
        break
      case 'ArrowLeft':
        // While a filter is being typed the arrows belong to the caret; the
        // headings stay reachable by mouse and by clearing the field.
        if (this._q.length > 0) return
        e.preventDefault()
        this._collapseFocused()
        break
      case 'ArrowRight':
        if (this._q.length > 0) return
        e.preventDefault()
        this._expandFocused()
        break
      case 'Enter':
        e.preventDefault()
        this._activateFocused()
        break
      case ' ':
      case 'x': {
        // These belong to the filter field until the user has navigated to a
        // task: `_focusRow` starts at -1 and every keystroke in the field
        // resets it, so typing "fix" or "buy milk" is unaffected. Once a task
        // IS focused they are its toggle keys — the same trade `Enter` already
        // makes, giving the key to the focused row rather than to the field.
        if (!this.editEnabled) return
        const hit = taskAt(this._groups, this._rows[this._focusRow])
        if (!hit) return
        e.preventDefault()
        if (e.key === 'x') {
          this._toggleCanceled(hit)
        } else {
          this._toggleDone(hit)
        }
        break
      }
    }
  }

  private _moveFocus(delta: number) {
    const rows = this._rows
    if (rows.length === 0) {
      this._focusRow = -1
      return
    }
    const next = this._focusRow + delta
    this._focusRow = Math.max(-1, Math.min(next, rows.length - 1))
    this._scrollFocusIntoView()
  }

  /** `←`: collapse the focused row's group and park focus on its heading. */
  private _collapseFocused() {
    const rows = this._rows
    const row = rows[this._focusRow]
    if (!row) return
    const group = this._groups[row.groupIndex]
    if (!group) return
    if (!this._collapsed.has(group.key)) {
      this._toggleCollapse(group.key)
    }
    // Recompute against the new collapse set: the focused task's row is gone.
    const headingRow = groupRowIndex(buildRows(this._groups, this._collapsed), row.groupIndex)
    this._focusRow = headingRow
    this._scrollFocusIntoView()
  }

  /** `→`: expand a collapsed heading, or step from an open heading into it. */
  private _expandFocused() {
    const rows = this._rows
    const row = rows[this._focusRow]
    if (!row || row.kind !== 'group') return
    const group = this._groups[row.groupIndex]
    if (!group) return
    if (this._collapsed.has(group.key)) {
      this._toggleCollapse(group.key)
      this._scrollFocusIntoView()
      return
    }
    if (group.tasks.length > 0) {
      this._focusRow = this._focusRow + 1
      this._scrollFocusIntoView()
    }
  }

  /** `Enter`: open a task, or toggle a heading — the same as clicking either. */
  private _activateFocused() {
    const rows = this._rows
    const row = rows[this._focusRow]
    if (!row) return
    if (row.kind === 'group') {
      const group = this._groups[row.groupIndex]
      if (group) this._toggleCollapse(group.key)
      return
    }
    const hit = taskAt(this._groups, row)
    if (hit) this._navigateTo(hit.url_path, hit.line)
  }

  private _navigateTo(urlPath: string, line: number) {
    const href = safeHref(`${this.resolveHref(urlPath)}#mbr-task-${line}`)
    // Plain navigation: the scroll/flash handler for the fragment is phase 9.
    window.location.assign(href)
  }

  private _activePaneElement(): Element | null {
    return (
      this.shadowRoot?.querySelector(
        this._activePaneIndex === 0 ? '.folder-pane-content' : '.results-list'
      ) ?? null
    )
  }

  private _scrollActivePane(pages: number) {
    const pane = this._activePaneElement()
    if (!pane) return
    // Full pages keep 50px of context, matching mbr-browse / mbr-search.
    const amount =
      Math.abs(pages) >= 1
        ? Math.sign(pages) * Math.max(pane.clientHeight - 50, 0)
        : pane.clientHeight * pages
    pane.scrollBy({ top: amount, behavior: 'smooth' })
  }

  private _scrollFocusIntoView() {
    void this.updateComplete.then(() => {
      const focused = this.shadowRoot?.querySelector('.focused')
      if (focused && typeof focused.scrollIntoView === 'function') {
        focused.scrollIntoView({ block: 'nearest' })
      }
    })
  }

  // ========================================
  // Render
  // ========================================

  override render() {
    return html`
      <div class="tasks-backdrop" @click=${() => this._close()}></div>
      <div class="tasks-container" role="dialog" aria-label="Tasks">
        ${this._renderFolderPane()} ${this._renderResultsPane()}
      </div>
    `
  }

  private _renderFolderPane(): TemplateResult {
    return html`
      <aside class="folder-pane ${this._activePaneIndex === 0 ? 'active' : ''}">
        <div class="pane-header">
          <h2>Folders</h2>
        </div>
        <div class="folder-pane-content">
          ${this._folderTree
            ? this._renderFolderNode(this._folderTree, 0)
            : html`<p class="pane-empty">No folders</p>`}
        </div>
      </aside>
    `
  }

  private _renderFolderNode(node: FolderTreeNode, depth: number): TemplateResult {
    const isRoot = node.path === '/'
    const hasChildren = node.children.length > 0
    const expanded = this._expandedFolders.has(node.path) || isRoot
    // Home is the "no scope" row, so it is selected whenever nothing else is.
    const selected = isRoot ? this._folder === null : this._folder === node.path

    return html`
      <div class="tree-item">
        <div class="tree-row ${selected ? 'selected' : ''}" style="padding-left: ${depth * 0.75 + 0.25}rem">
          ${hasChildren
            ? html`<button
                class="tree-toggle"
                aria-expanded=${expanded}
                aria-label=${expanded ? 'Collapse folder' : 'Expand folder'}
                @click=${(e: Event) => {
                  e.stopPropagation()
                  this._toggleFolderExpansion(node.path)
                }}
              >
                ${expanded ? '▼' : '▶'}
              </button>`
            : html`<span class="tree-spacer"></span>`}
          <button class="tree-label" @click=${() => this._selectFolder(isRoot ? null : node.path)}>
            <span class="folder-icon" aria-hidden="true">📁</span>
            <span class="label-text">${node.name}</span>
            <span class="label-count">${node.count}</span>
          </button>
        </div>
      </div>
      ${expanded
        ? html`${node.children.map((child) => this._renderFolderNode(child, depth + 1))}`
        : nothing}
    `
  }

  private _renderResultsPane(): TemplateResult {
    return html`
      <div class="results-pane ${this._activePaneIndex === 1 ? 'active' : ''}">
        <div class="filter-bar">
          <input
            id="tasks-filter"
            type="search"
            placeholder="Filter tasks… (#tag for tags)"
            .value=${this._q}
            @input=${this._handleFilterInput}
            autocomplete="off"
            spellcheck="false"
            aria-label="Filter tasks"
          />
          <button
            class="filter-button ${this._filtersOpen ? 'open' : ''}"
            @click=${() => (this._filtersOpen = !this._filtersOpen)}
            aria-expanded=${this._filtersOpen}
            aria-label="Filter options"
            title="Filter options"
          >
            ⚙
          </button>
          <button class="close-button" @click=${() => this._close()} aria-label="Close">✕</button>
        </div>
        ${this._filtersOpen ? this._renderFilterPopover() : nothing} ${this._renderModeTabs()}
        ${this._renderStatusLine()} ${this._renderNotice()}
        <div class="results-list">${this._renderResults()}</div>
        ${this._renderFooter()}
      </div>
    `
  }

  private _renderFilterPopover(): TemplateResult {
    return html`
      <div class="filter-popover" role="group" aria-label="Filter options">
        <fieldset>
          <legend>Status</legend>
          ${STATUS_OPTIONS.map(
            (option) => html`
              <label>
                <input
                  type="checkbox"
                  .checked=${this._statuses.includes(option.value)}
                  @change=${(e: Event) =>
                    this._toggleStatus(option.value, e.target as HTMLInputElement)}
                />
                <span>${option.label}</span>
              </label>
            `
          )}
        </fieldset>
        <fieldset>
          <legend>Priority</legend>
          ${PRIORITY_OPTIONS.map(
            (option) => html`
              <label>
                <input
                  type="checkbox"
                  .checked=${this._priorities.includes(option.value)}
                  @change=${() => this._togglePriority(option.value)}
                />
                <span>${option.label}</span>
              </label>
            `
          )}
        </fieldset>
        <fieldset>
          <legend>Due</legend>
          <select
            aria-label="Due date filter"
            @change=${(e: Event) => this._setDue((e.target as HTMLSelectElement).value as DueFilter)}
          >
            ${DUE_OPTIONS.map(
              (option) => html`
                <option value=${option.value} ?selected=${this._due === option.value}>
                  ${option.label}
                </option>
              `
            )}
          </select>
        </fieldset>
      </div>
    `
  }

  private _renderModeTabs(): TemplateResult {
    const tab = (mode: TaskMode, icon: string, label: string) => html`
      <button
        class="mode-tab ${this._mode === mode ? 'active' : ''}"
        role="tab"
        aria-selected=${this._mode === mode}
        @click=${() => this._setMode(mode)}
      >
        <span aria-hidden="true">${icon}</span> ${label}
      </button>
    `
    return html`
      <div class="mode-tabs" role="tablist" aria-label="Grouping mode">
        ${tab('category', '▤', 'Category')} ${tab('calendar', '▦', 'Calendar')}
      </div>
    `
  }

  private _renderStatusLine(): TemplateResult | typeof nothing {
    if (this._error) return nothing
    const response = this._response
    if (!response) return nothing
    const count = response.total_matches
    return html`
      <div class="results-meta">
        ${count} task${count === 1 ? '' : 's'} in ${response.duration_ms}ms
        ${response.scan_in_progress
          ? html`<span class="scanning" aria-live="polite">· still indexing…</span>`
          : nothing}
        ${this._loading ? html`<span class="loading" aria-busy="true">· loading…</span>` : nothing}
      </div>
    `
  }

  /**
   * A failed write's reason.
   *
   * Kept separate from `_error` (which reports a failed *query*) because the
   * two have different lifetimes: a query error replaces the results, while
   * this sits above results that are still perfectly good.
   */
  private _renderNotice(): TemplateResult | typeof nothing {
    if (!this._notice) return nothing
    return html`
      <div class="results-notice" role="status">
        ${this._notice}
        <button class="notice-dismiss" @click=${() => (this._notice = null)} aria-label="Dismiss">
          ✕
        </button>
      </div>
    `
  }

  private _renderResults(): TemplateResult | TemplateResult[] {
    if (this._error) {
      return html`<div class="results-error">${this._error}</div>`
    }
    if (!this._response) {
      return html`<div class="results-empty" aria-busy="true">Loading tasks…</div>`
    }
    const groups = this._groups
    if (groups.length === 0) {
      return html`<div class="results-empty">No tasks match these filters.</div>`
    }

    const rows = this._rows
    // One pass over the row list keeps the rendered order and the keyboard's
    // idea of it identical by construction.
    return rows.map((row, index) =>
      row.kind === 'group'
        ? this._renderGroupHeading(groups[row.groupIndex], index)
        : this._renderTaskRow(groups[row.groupIndex], row.taskIndex, index)
    )
  }

  private _renderGroupHeading(group: DisplayGroup, rowIndex: number): TemplateResult {
    const collapsed = this._collapsed.has(group.key)
    const focused = rowIndex === this._focusRow
    const percent = progressPercent(group.done, group.total)
    // Calendar date buckets arrive ISO-labelled on purpose so the locale is
    // decided here rather than hard-coded to English on the server.
    const label =
      group.level === 1 && group.date
        ? formatDateHeading(group.date, this.today, this.locale)
        : group.label

    return html`
      <button
        class="group-heading level-${group.level} ${focused ? 'focused' : ''}"
        aria-expanded=${!collapsed}
        @mouseenter=${() => (this._focusRow = rowIndex)}
        @click=${() => this._toggleCollapse(group.key)}
      >
        <span class="group-twisty" aria-hidden="true">${collapsed ? '▶' : '▼'}</span>
        <span class="group-titles">
          <span class="group-label">${label}</span>
          ${group.sublabel ? html`<span class="group-sublabel">${group.sublabel}</span>` : nothing}
        </span>
        ${group.showProgress
          ? html`
              <span class="group-progress">
                <span class="group-count">${group.done}/${group.total}</span>
                <span
                  class="progress-track"
                  role="progressbar"
                  aria-valuenow=${group.done}
                  aria-valuemin="0"
                  aria-valuemax=${group.total}
                >
                  <span class="progress-fill" style="width: ${percent}%"></span>
                </span>
              </span>
            `
          : nothing}
      </button>
    `
  }

  private _renderTaskRow(
    group: DisplayGroup,
    taskIndex: number,
    rowIndex: number
  ): TemplateResult {
    const hit = group.tasks[taskIndex]
    const editable = this.editEnabled && this.toggleTask !== null
    return renderTaskCard({
      hit,
      status: this._statusOf(hit),
      href: safeHref(taskHref(hit, this.resolveHref)),
      focused: rowIndex === this._focusRow,
      editable,
      today: this.today,
      locale: this.locale,
      onFocus: () => (this._focusRow = rowIndex),
      onActivate: () => this._navigateTo(hit.url_path, hit.line),
      onToggle: editable ? () => this._toggleDone(hit) : undefined,
      onCancel: editable ? () => this._toggleCanceled(hit) : undefined,
    })
  }

  private _renderFooter(): TemplateResult {
    return html`
      <div class="tasks-footer">
        <span class="footer-hint">
          <kbd>^n</kbd><kbd>^p</kbd> navigate <kbd>↵</kbd> open
          ${this.editEnabled
            ? html`<kbd>space</kbd> done <kbd>x</kbd> cancel `
            : nothing}<kbd>←</kbd><kbd>→</kbd> collapse <kbd>⇥</kbd> pane <kbd>^d</kbd><kbd>^u</kbd>
          scroll <kbd>esc</kbd> close
        </span>
      </div>
    `
  }

  // ========================================
  // Styles
  // ========================================

  static override styles = css`
    :host {
      display: contents;
    }

    /* ---- Shell: same backdrop/container conventions as <mbr-browse> ---- */

    .tasks-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.4);
      z-index: 1000;
      animation: fadeIn 0.2s ease;
    }

    @keyframes fadeIn {
      from {
        opacity: 0;
      }
      to {
        opacity: 1;
      }
    }

    .tasks-container {
      position: fixed;
      left: 0;
      top: 0;
      height: 100%;
      max-width: 100vw;
      display: flex;
      z-index: 1001;
      animation: slideIn 0.25s ease;
    }

    @keyframes slideIn {
      from {
        transform: translateX(-100%);
      }
      to {
        transform: translateX(0);
      }
    }

    .folder-pane {
      width: 240px;
      height: 100%;
      background: var(--pico-background-color, #fff);
      border-right: 1px solid var(--pico-muted-border-color, #eee);
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }

    .results-pane {
      width: 520px;
      max-width: calc(100vw - 240px);
      height: 100%;
      background: var(--pico-card-background-color, #f8f9fa);
      border-right: 1px solid var(--pico-muted-border-color, #eee);
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }

    /* The active pane owns Ctrl+d/u/f/b; mark it so that is visible. */
    .folder-pane.active,
    .results-pane.active {
      box-shadow: inset 3px 0 0 var(--pico-primary, #0d6efd);
    }

    .pane-header {
      display: flex;
      align-items: center;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .pane-header h2 {
      margin: 0;
      font-size: 0.8rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-muted-color, #666);
    }

    .folder-pane-content {
      flex: 1;
      overflow-y: auto;
      padding: 0.5rem 0;
    }

    .pane-empty,
    .results-empty,
    .results-error {
      padding: 2rem 1rem;
      text-align: center;
      color: var(--pico-muted-color, #666);
    }

    .results-error {
      color: var(--pico-del-color, #dc3545);
    }

    /* ---- Folder tree ---- */

    .tree-row {
      display: flex;
      align-items: center;
      padding: 0.3rem 0.5rem;
      border-radius: 4px;
      min-width: 0;
    }

    .tree-row.selected {
      background: var(--pico-primary-background, #e3f2fd);
      color: var(--pico-primary-inverse, #fff);
    }

    .tree-row.selected .tree-label,
    .tree-row.selected .label-count {
      color: var(--pico-primary-inverse, #fff);
    }

    .tree-row:hover {
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .tree-toggle {
      width: 1rem;
      height: 1rem;
      background: transparent;
      border: none;
      cursor: pointer;
      font-size: 0.6rem;
      color: var(--pico-muted-color, #999);
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;
      padding: 0;
    }

    .tree-spacer {
      width: 1rem;
      flex-shrink: 0;
    }

    .tree-label {
      flex: 1;
      display: flex;
      align-items: center;
      gap: 0.35rem;
      background: transparent;
      border: none;
      cursor: pointer;
      text-align: left;
      padding: 0;
      color: var(--pico-color, #333);
      font-size: 0.85rem;
      min-width: 0;
      overflow: hidden;
    }

    .label-text {
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .label-count {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
      padding-right: 0.25rem;
    }

    /* ---- Filter bar ---- */

    .filter-bar {
      display: flex;
      align-items: center;
      gap: 0.4rem;
      padding: 0.6rem 0.75rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .filter-bar input[type='search'] {
      flex: 1;
      min-width: 0;
      margin: 0;
      padding: 0.35rem 0.6rem;
      font-size: 0.9rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
    }

    .filter-button,
    .close-button {
      background: transparent;
      border: none;
      cursor: pointer;
      font-size: 1rem;
      line-height: 1;
      padding: 0.3rem 0.4rem;
      border-radius: 4px;
      color: var(--pico-muted-color, #999);
    }

    .filter-button:hover,
    .close-button:hover,
    .filter-button.open {
      color: var(--pico-color, #333);
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .filter-popover {
      display: flex;
      flex-wrap: wrap;
      gap: 0.75rem;
      padding: 0.6rem 0.75rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      background: var(--pico-background-color, #fff);
      flex-shrink: 0;
    }

    .filter-popover fieldset {
      margin: 0;
      padding: 0;
      border: none;
    }

    .filter-popover legend {
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-muted-color, #666);
      padding: 0;
      margin-bottom: 0.2rem;
    }

    .filter-popover label {
      display: flex;
      align-items: center;
      gap: 0.3rem;
      font-size: 0.8rem;
      color: var(--pico-color, #333);
      margin: 0;
    }

    .filter-popover input[type='checkbox'] {
      margin: 0;
    }

    .filter-popover select {
      margin: 0;
      font-size: 0.8rem;
      padding: 0.2rem 0.4rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
    }

    /* ---- Mode tabs ---- */

    .mode-tabs {
      display: flex;
      justify-content: center;
      gap: 0.35rem;
      padding: 0.4rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .mode-tab {
      background: transparent;
      border: 1px solid transparent;
      border-radius: 999px;
      cursor: pointer;
      font-size: 0.8rem;
      padding: 0.2rem 0.8rem;
      color: var(--pico-muted-color, #666);
    }

    .mode-tab:hover {
      border-color: var(--pico-muted-border-color, #ccc);
    }

    .mode-tab.active {
      background: var(--pico-primary-background, #e3f2fd);
      color: var(--pico-primary-inverse, #fff);
    }

    .results-meta {
      padding: 0.3rem 0.75rem;
      font-size: 0.72rem;
      color: var(--pico-muted-color, #999);
      flex-shrink: 0;
    }

    .results-list {
      flex: 1;
      overflow-y: auto;
      padding: 0.25rem 0.5rem 0.75rem;
    }

    /* A refused write. Sits above results that are still valid, so it is a
     * strip rather than a replacement for the list. */
    .results-notice {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      margin: 0 0.5rem;
      padding: 0.35rem 0.5rem;
      border-radius: 4px;
      font-size: 0.78rem;
      color: var(--pico-del-color, #dc3545);
      background: var(--mbr-task-chip-bg, rgba(115, 130, 140, 0.1));
      flex-shrink: 0;
    }

    .notice-dismiss {
      margin-inline-start: auto;
      background: transparent;
      border: none;
      cursor: pointer;
      font-size: 0.75rem;
      line-height: 1;
      padding: 0.1rem 0.2rem;
      color: inherit;
    }

    /* ---- Group headings ---- */

    .group-heading {
      display: flex;
      align-items: center;
      gap: 0.4rem;
      width: 100%;
      background: transparent;
      border: none;
      border-radius: 4px;
      cursor: pointer;
      text-align: left;
      padding: 0.35rem 0.4rem;
      margin-top: 0.4rem;
      color: var(--pico-color, #333);
    }

    .group-heading.level-1 {
      margin-inline-start: 0.9rem;
      font-size: 0.85em;
    }

    .group-heading:hover,
    .group-heading.focused {
      background: var(--pico-secondary-background, #f0f0f0);
    }

    .group-heading.focused {
      outline: 2px solid var(--pico-primary, #0d6efd);
      outline-offset: -2px;
    }

    .group-twisty {
      font-size: 0.6rem;
      color: var(--pico-muted-color, #999);
      flex-shrink: 0;
    }

    .group-titles {
      flex: 1;
      min-width: 0;
      display: flex;
      flex-direction: column;
    }

    .group-label {
      font-weight: 600;
      font-size: 0.9em;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .group-sublabel {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .group-progress {
      display: flex;
      align-items: center;
      gap: 0.4rem;
      flex-shrink: 0;
    }

    .group-count {
      font-size: 0.72rem;
      color: var(--pico-muted-color, #999);
      font-variant-numeric: tabular-nums;
    }

    .progress-track {
      display: inline-block;
      width: 3.5rem;
      height: 0.35rem;
      border-radius: 999px;
      background: var(--pico-muted-border-color, #e0e0e0);
      overflow: hidden;
    }

    .progress-fill {
      display: block;
      height: 100%;
      background: var(--pico-primary, #0d6efd);
    }

    /* ---- Task cards ----
     *
     * The chip/pill/dot rules below are a deliberate mirror of the ones in
     * templates/theme.css. Those cannot reach into this shadow root, but the
     * --mbr-task-* custom properties they are built from DO inherit across the
     * boundary, so restating the rules against the same tokens keeps the panel
     * and the rendered page looking like one product — theme switches and the
     * dark-mode overrides included.
     */

    .task-card {
      display: flex;
      align-items: baseline;
      gap: 0.4rem;
      padding: 0.3rem 0.4rem;
      border-radius: 4px;
      cursor: pointer;
    }

    .task-card:hover,
    .task-card.focused {
      background: var(--pico-secondary-background, #f0f0f0);
    }

    .task-card.focused {
      outline: 2px solid var(--pico-primary, #0d6efd);
      outline-offset: -2px;
    }

    .task-body {
      flex: 1;
      min-width: 0;
      display: flex;
      flex-wrap: wrap;
      align-items: baseline;
      gap: 0.35rem;
    }

    .task-link {
      color: var(--pico-color, #333);
      text-decoration: none;
      font-size: 0.9rem;
      overflow-wrap: anywhere;
    }

    .task-link:hover {
      text-decoration: underline;
    }

    .task-chips {
      display: inline-flex;
      flex-wrap: wrap;
      align-items: baseline;
      gap: 0.3rem;
    }

    .mbr-task-check {
      vertical-align: middle;
      margin: 0;
      flex-shrink: 0;
    }

    /* Only a box that can actually be written to invites a click. */
    .mbr-task-check.mbr-task-editable {
      cursor: pointer;
    }

    .mbr-task-canceled {
      text-decoration: line-through;
      color: var(--mbr-task-canceled-color, var(--pico-muted-color, #5c6b73));
    }

    .mbr-task-pri,
    .mbr-task-pri-spacer {
      display: inline-block;
      width: var(--mbr-task-pri-size, 0.55em);
      height: var(--mbr-task-pri-size, 0.55em);
      border-radius: 50%;
      flex-shrink: 0;
    }

    .mbr-task-pri-high {
      background: var(--mbr-task-pri-high-color, #f57c00);
    }

    .mbr-task-pri-urgent {
      background: var(--mbr-task-pri-urgent-color, #c62828);
    }

    .mbr-task-tag {
      background: var(--mbr-task-tag-bg, rgba(115, 130, 140, 0.16));
      color: var(--mbr-task-tag-color, var(--pico-muted-color, #5c6b73));
      border-radius: var(--mbr-task-chip-radius, 0.5em);
      padding: 0.05em 0.5em;
      font-size: 0.72rem;
      white-space: nowrap;
    }

    .mbr-task-due,
    .mbr-task-completed {
      background: var(--mbr-task-chip-bg, rgba(115, 130, 140, 0.1));
      color: var(--mbr-task-chip-color, var(--pico-muted-color, #5c6b73));
      border-radius: var(--mbr-task-chip-radius, 0.5em);
      padding: 0.05em 0.5em;
      font-size: 0.72rem;
      white-space: nowrap;
    }

    .mbr-task-due::before,
    .mbr-task-completed::before {
      margin-inline-end: 0.3em;
    }

    .mbr-task-due::before {
      content: '\\1F5D3\\FE0E'; /* 🗓 calendar, text presentation */
    }

    .mbr-task-completed::before {
      content: '\\2713'; /* ✓ */
    }

    /*
     * Overdue is decided here and nowhere else: the server renders documents
     * without it on purpose, because a cached page would be wrong the next
     * morning. Reuses the urgent-priority color so the two read as one scale.
     */
    .mbr-task-due.mbr-task-overdue {
      color: var(--mbr-task-pri-urgent-color, #c62828);
      font-weight: 600;
    }

    /* ---- Footer ---- */

    .tasks-footer {
      padding: 0.4rem 0.75rem;
      border-top: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .footer-hint {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
    }

    kbd {
      display: inline-block;
      padding: 0.05rem 0.3rem;
      margin-right: 0.1rem;
      font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
      font-size: 0.68rem;
      color: var(--pico-color, #333);
      background: var(--pico-secondary-background, #f5f5f5);
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 3px;
    }

    /* ---- Responsive ---- */

    @media (max-width: 900px) {
      .folder-pane {
        width: 180px;
      }

      .results-pane {
        width: calc(100vw - 180px);
      }
    }

    @media (min-width: 1400px) {
      .folder-pane {
        width: 300px;
      }

      .results-pane {
        width: 620px;
      }
    }
  `
}
