/**
 * One task card in the results pane.
 *
 * A plain render function rather than its own custom element: the panel already
 * owns a shadow root, and keeping the card there means one copy of the
 * `--mbr-task-*` styles instead of one per card.
 *
 * The visual vocabulary — priority dot, tag pill, date chip — is deliberately
 * the same one `templates/theme.css` gives rendered documents. Those rules
 * cannot cross into a shadow root, but the custom properties they are built
 * from do, so the panel restates the rules against the same tokens and inherits
 * every theme and dark-mode override for free.
 */
import { html, nothing, type TemplateResult } from 'lit'
import { chipDatetime, formatChipLabel, isOverdue, priorityDot } from './task-format.js'
import type { TaskHit, TaskStatus } from './types.js'

/** Indent per nesting level for a subtask, in rem. */
const DEPTH_INDENT_REM = 0.9

/** Cap on the rendered indent, so a deeply nested task keeps its text readable. */
const MAX_DEPTH = 6

export interface TaskCardOptions {
  hit: TaskHit
  /**
   * The status to draw, which is not always `hit.status`: while a toggle is in
   * flight the panel passes the optimistic one, so the box moves on click
   * rather than a round trip later.
   */
  status: TaskStatus
  /** Deep link to the task's line (`…/page/#mbr-task-42`). */
  href: string
  focused: boolean
  /** Whether the checkbox accepts clicks (editing on, and a toggler injected). */
  editable: boolean
  /** Today, for runtime overdue marking. */
  today: Date
  /** Locale override; tests pin this so the assertions are not machine-dependent. */
  locale?: string
  onFocus: () => void
  onActivate: (e: Event) => void
  /** Left click: done ↔ open. Never called unless `editable`. */
  onToggle?: () => void
  /** Right click: canceled ↔ open. Never called unless `editable`. */
  onCancel?: () => void
}

export function renderTaskCard(options: TaskCardOptions): TemplateResult {
  const { hit, status, href, focused, editable, today, locale } = options
  const dot = priorityDot(hit.priority)
  const canceled = status === 'canceled'
  const indent = Math.min(hit.depth ?? 0, MAX_DEPTH) * DEPTH_INDENT_REM

  return html`
    <div
      class="task-card ${focused ? 'focused' : ''}"
      style="margin-inline-start: ${indent}rem"
      data-line=${hit.line}
      @mouseenter=${options.onFocus}
      @click=${options.onActivate}
    >
      ${dot
        ? html`<span
            class="mbr-task-pri ${dot.className}"
            role="img"
            aria-label=${dot.label}
            title=${dot.label}
          ></span>`
        : html`<span class="mbr-task-pri-spacer"></span>`}
      <input
        type="checkbox"
        class="mbr-task-check ${editable ? 'mbr-task-editable' : ''}"
        data-mbr-task-line=${hit.line}
        data-mbr-task-status=${status}
        .checked=${status === 'done'}
        ?disabled=${!editable}
        aria-label=${hit.text}
        title=${editable ? 'Click to complete · right-click to cancel' : nothing}
        @click=${(e: Event) => {
          // The card behind opens the file; a click on the box must not.
          e.stopPropagation()
          if (!editable) return
          // Cancel the browser's own flip and let the render decide instead.
          // Safe here (unlike in the document, see `mbr-task-doc.ts`) because
          // `checked` is a PROPERTY binding: Lit re-commits it from `status`
          // on the very next render, after the cancellation has restored it.
          e.preventDefault()
          options.onToggle?.()
        }}
        @contextmenu=${(e: Event) => {
          if (!editable) return
          e.preventDefault()
          e.stopPropagation()
          options.onCancel?.()
        }}
      />
      <div class="task-body">
        <a
          class="task-link ${canceled ? 'mbr-task-canceled' : ''}"
          href=${href}
          @click=${(e: Event) => e.stopPropagation()}
          >${hit.text}</a
        >
        ${renderChips(hit, status, today, locale)}
      </div>
    </div>
  `
}

function renderChips(
  hit: TaskHit,
  status: TaskStatus,
  today: Date,
  locale?: string
): TemplateResult | typeof nothing {
  const tags = hit.tags ?? []
  const hasChips = tags.length > 0 || hit.due !== null || hit.done !== null
  if (!hasChips) return nothing

  const overdue = status === 'open' && isOverdue(hit.due, today)

  return html`
    <span class="task-chips">
      ${tags.map((tag) => html`<span class="mbr-task-tag">#${tag}</span>`)}
      ${hit.due
        ? html`<time
            class="mbr-task-due ${overdue ? 'mbr-task-overdue' : ''}"
            datetime=${chipDatetime(hit.due, hit.due_has_time)}
            title=${overdue ? 'Overdue' : 'Due'}
            >${formatChipLabel(hit.due, hit.due_has_time, locale)}</time
          >`
        : nothing}
      ${hit.done
        ? html`<time
            class="mbr-task-completed"
            datetime=${chipDatetime(hit.done, hit.done_has_time)}
            title="Completed"
            >${formatChipLabel(hit.done, hit.done_has_time, locale)}</time
          >`
        : nothing}
    </span>
  `
}
