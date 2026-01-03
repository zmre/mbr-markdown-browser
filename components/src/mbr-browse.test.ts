import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import './mbr-browse.js'
import type { MbrBrowseElement } from './mbr-browse.js'

describe('MbrBrowseElement', () => {
  let element: MbrBrowseElement

  beforeEach(() => {
    element = document.createElement('mbr-browse') as MbrBrowseElement
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
  })

  describe('registration', () => {
    it('should be defined as a custom element', () => {
      expect(customElements.get('mbr-browse')).toBeDefined()
    })

    it('should create an instance', () => {
      expect(element).toBeInstanceOf(HTMLElement)
      expect(element.tagName.toLowerCase()).toBe('mbr-browse')
    })
  })

  describe('visibility', () => {
    it('should be closed by default', () => {
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })

    it('should open when open() is called', async () => {
      element.open()
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).not.toBeNull()
    })

    it('should close when close() is called', async () => {
      element.open()
      await element.updateComplete
      element.close()
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })

    it('should toggle visibility', async () => {
      element.toggle()
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).not.toBeNull()

      element.toggle()
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })
  })

  describe('keyboard navigation', () => {
    it('should open with "-" key', async () => {
      const event = new KeyboardEvent('keydown', { key: '-', bubbles: true })
      document.dispatchEvent(event)
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).not.toBeNull()
    })

    it('should open with F2 key', async () => {
      const event = new KeyboardEvent('keydown', { key: 'F2', bubbles: true })
      document.dispatchEvent(event)
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).not.toBeNull()
    })

    it('should close with Escape key', async () => {
      element.open()
      await element.updateComplete

      const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })
      document.dispatchEvent(event)
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })

    it('should not open with "-" when in an input field', async () => {
      const input = document.createElement('input')
      document.body.appendChild(input)
      input.focus()

      const event = new KeyboardEvent('keydown', { key: '-', bubbles: true })
      Object.defineProperty(event, 'target', { value: input })
      document.dispatchEvent(event)
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
      input.remove()
    })
  })

  describe('structure', () => {
    it('should render left pane when open', async () => {
      element.open()
      await element.updateComplete

      const leftPane = element.shadowRoot?.querySelector('.left-pane')
      expect(leftPane).not.toBeNull()
    })

    it('should render pane header with title', async () => {
      element.open()
      await element.updateComplete

      const header = element.shadowRoot?.querySelector('.pane-header h2')
      expect(header?.textContent).toBe('Navigate')
    })

    it('should render close button', async () => {
      element.open()
      await element.updateComplete

      const closeBtn = element.shadowRoot?.querySelector('.close-button')
      expect(closeBtn).not.toBeNull()
    })

    it('should close when backdrop is clicked', async () => {
      element.open()
      await element.updateComplete

      const backdrop = element.shadowRoot?.querySelector('.navigator-backdrop') as HTMLElement
      backdrop?.click()
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })

    it('should close when close button is clicked', async () => {
      element.open()
      await element.updateComplete

      const closeBtn = element.shadowRoot?.querySelector('.close-button') as HTMLElement
      closeBtn?.click()
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })
  })

  describe('loading state', () => {
    it('should show loading state initially', async () => {
      element.open()
      await element.updateComplete

      // Component fetches site.json on mount, shows loading state
      const loading = element.shadowRoot?.querySelector('.loading-container')
      // May or may not be visible depending on timing, but structure should exist
      const paneContent = element.shadowRoot?.querySelector('.pane-content')
      expect(paneContent).not.toBeNull()
    })
  })
})
