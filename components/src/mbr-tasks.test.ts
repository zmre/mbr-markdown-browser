import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import './mbr-tasks.js'
import { setTasksChunkImporter, type MbrTasksElement } from './mbr-tasks.js'
import { OVERLAY_TAGS, isAnyOverlayOpen } from './overlay.js'

/** Let the chunk-load promise chain settle. */
async function settle(element: MbrTasksElement): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await element.updateComplete
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

function press(key: string, target: EventTarget = document): void {
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, composed: true }))
}

describe('MbrTasksElement', () => {
  let element: MbrTasksElement
  let importer: ReturnType<typeof vi.fn<() => Promise<unknown>>>

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, tasksEnabled: true }
    importer = vi.fn<() => Promise<unknown>>().mockResolvedValue({})
    setTasksChunkImporter(importer)
    element = document.createElement('mbr-tasks') as MbrTasksElement
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
    window.__MBR_CONFIG__ = undefined
    window.frontmatter = undefined
    setTasksChunkImporter(() => Promise.reject(new Error('unset test importer')))
  })

  describe('registration', () => {
    it('is defined as a custom element', () => {
      expect(customElements.get('mbr-tasks')).toBeDefined()
    })

    it('is listed in OVERLAY_TAGS so mbr-keys suppresses bare-letter shortcuts', () => {
      expect(OVERLAY_TAGS).toContain('mbr-tasks')
    })

    it('reports itself through the MbrOverlay contract', async () => {
      expect(element.isOpen).toBe(false)
      expect(isAnyOverlayOpen()).toBe(false)
      element.open()
      await settle(element)
      expect(element.isOpen).toBe(true)
      expect(isAnyOverlayOpen()).toBe(true)
      element.close()
      expect(element.isOpen).toBe(false)
    })
  })

  describe('availability gating', () => {
    it('renders nothing at all when the task browser is disabled', async () => {
      window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, tasksEnabled: false }
      const disabled = document.createElement('mbr-tasks') as MbrTasksElement
      document.body.appendChild(disabled)
      await disabled.updateComplete

      expect(disabled.shadowRoot?.querySelector('button')).toBeNull()
      expect(disabled.shadowRoot?.textContent?.trim()).toBe('')
      disabled.remove()
    })

    it('renders nothing in a static build, where tasksEnabled is absent', async () => {
      window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../' }
      const staticEl = document.createElement('mbr-tasks') as MbrTasksElement
      document.body.appendChild(staticEl)
      await staticEl.updateComplete

      expect(staticEl.shadowRoot?.querySelector('button')).toBeNull()
      staticEl.remove()
    })

    it('renders a clipboard trigger with the shortcut in its title when enabled', async () => {
      await element.updateComplete
      const button = element.shadowRoot?.querySelector('button.tasks-trigger')
      expect(button).not.toBeNull()
      expect(button?.getAttribute('title')).toBe('Tasks (t)')
      // Feather-style icon: stroked, not filled, so it tracks the text color.
      const svg = button?.querySelector('svg')
      expect(svg?.getAttribute('stroke')).toBe('currentColor')
      expect(svg?.getAttribute('fill')).toBe('none')
    })
  })

  describe('the "t" shortcut', () => {
    it('opens the panel', async () => {
      press('t')
      await settle(element)
      expect(element.isOpen).toBe(true)
      expect(element.shadowRoot?.querySelector('mbr-tasks-panel')).not.toBeNull()
    })

    it('is ignored when the task browser is disabled', async () => {
      window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, tasksEnabled: false }
      press('t')
      await settle(element)
      expect(element.isOpen).toBe(false)
      expect(importer).not.toHaveBeenCalled()
    })

    it('is ignored while typing in an input', async () => {
      const input = document.createElement('input')
      document.body.appendChild(input)
      input.focus()
      press('t', input)
      await settle(element)
      expect(element.isOpen).toBe(false)
      input.remove()
    })

    it('is ignored for a text field inside a shadow root', async () => {
      // The document-level listener sees the event retargeted to the shadow
      // HOST, so this only works because isInputTarget uses composedPath.
      const host = document.createElement('div')
      const root = host.attachShadow({ mode: 'open' })
      const input = document.createElement('input')
      root.appendChild(input)
      document.body.appendChild(host)

      press('t', input)
      await settle(element)
      expect(element.isOpen).toBe(false)
      host.remove()
    })

    it('is ignored while another modal owns the keyboard', async () => {
      // isModalOpen() also reports the info panel's checkbox.
      const toggle = document.createElement('input')
      toggle.type = 'checkbox'
      toggle.id = 'info-panel-toggle'
      toggle.checked = true
      document.body.appendChild(toggle)

      press('t')
      await settle(element)
      expect(element.isOpen).toBe(false)
      toggle.remove()
    })

    it('is ignored with a modifier held', async () => {
      for (const modifier of ['ctrlKey', 'metaKey', 'altKey', 'shiftKey'] as const) {
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 't', [modifier]: true }))
      }
      await settle(element)
      expect(element.isOpen).toBe(false)
    })

    it('does not close an open panel, so "t" can be typed into the filter', async () => {
      element.open()
      await settle(element)
      // The panel's own filter input is focused; a bare `t` must reach it.
      press('t')
      await settle(element)
      expect(element.isOpen).toBe(true)
    })
  })

  describe('chunk loading', () => {
    it('imports the chunk once, no matter how often the panel is reopened', async () => {
      element.open()
      await settle(element)
      expect(importer).toHaveBeenCalledTimes(1)
      expect(element.shadowRoot?.querySelector('mbr-tasks-panel')).not.toBeNull()

      element.close()
      await element.updateComplete
      element.open()
      await settle(element)
      expect(importer).toHaveBeenCalledTimes(1)
    })

    it('closes again and shows no panel when the import fails', async () => {
      setTasksChunkImporter(vi.fn().mockRejectedValue(new Error('offline')))
      element.open()
      await settle(element)
      expect(element.isOpen).toBe(false)
      expect(element.shadowRoot?.querySelector('mbr-tasks-panel')).toBeNull()
    })

    it('injects the services the chunk cannot import for itself', async () => {
      window.__MBR_CONFIG__ = {
        serverMode: true,
        guiMode: false,
        tasksEnabled: true,
        editEnabled: true,
      }
      window.frontmatter = { markdown_source: 'docs/notes.md' }
      element.open()
      await settle(element)

      // The panel element is not upgraded here (the chunk is stubbed), so
      // these land as plain properties — which is exactly how Lit would set
      // them, and what the panel reads.
      const panel = element.shadowRoot?.querySelector('mbr-tasks-panel') as unknown as {
        endpoint: string
        editEnabled: boolean
        toggleTask: unknown
        resolveHref: unknown
        currentPath: string | null
      }
      expect(panel.endpoint).toBe('/.mbr/tasks')
      expect(panel.editEnabled).toBe(true)
      expect(typeof panel.toggleTask).toBe('function')
      expect(typeof panel.resolveHref).toBe('function')
      // The page the panel opens scoped to and pins first; the chunk cannot
      // read `window.frontmatter` through `task-toggle.ts` for itself.
      expect(panel.currentPath).toBe('docs/notes.md')
    })

    it('injects a null current path from a page that is not a file', async () => {
      // Section and home pages render no `markdown_source`, and the panel then
      // opens exactly as it always has: unscoped and unpinned.
      element.open()
      await settle(element)

      const panel = element.shadowRoot?.querySelector('mbr-tasks-panel') as unknown as {
        currentPath: string | null
      }
      expect(panel.currentPath).toBeNull()
    })

    it('does not offer toggling when editing is off', async () => {
      element.open()
      await settle(element)

      const panel = element.shadowRoot?.querySelector('mbr-tasks-panel') as unknown as {
        editEnabled: boolean
      }
      expect(panel.editEnabled).toBe(false)
    })

    it('closes when the panel asks to be closed', async () => {
      element.open()
      await settle(element)
      const panel = element.shadowRoot?.querySelector('mbr-tasks-panel')
      panel?.dispatchEvent(new CustomEvent('mbr-tasks-close'))
      await element.updateComplete
      expect(element.isOpen).toBe(false)
    })
  })

  describe('the trigger button', () => {
    it('toggles the panel', async () => {
      const button = element.shadowRoot?.querySelector('button.tasks-trigger') as HTMLButtonElement
      button.click()
      await settle(element)
      expect(element.isOpen).toBe(true)

      button.click()
      await element.updateComplete
      expect(element.isOpen).toBe(false)
    })
  })
})
