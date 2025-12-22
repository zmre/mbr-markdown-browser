import { LitElement, css, html } from 'lit';
import { customElement, state } from 'lit/decorators.js';

interface FileChangeEvent {
  path: string;
  relative_path: string;
  event: 'modified' | 'created' | 'deleted';
}

/**
 * Live reload component that monitors file changes via WebSocket
 * and automatically reloads the page when relevant files change.
 */
@customElement('mbr-live-reload')
export class MbrLiveReloadElement extends LitElement {
  @state()
  private _showNotification = false;

  private _ws: WebSocket | null = null;
  private _reconnectAttempts = 0;
  private _maxReconnectAttempts = 5;
  private _reconnectDelay = 1000; // Start with 1 second
  private _currentMarkdownFile: string | null = null;

  static override styles = css`
    :host {
      position: fixed;
      bottom: 20px;
      right: 20px;
      z-index: 10000;
      pointer-events: none;
    }

    .notification {
      background: rgba(0, 0, 0, 0.85);
      color: white;
      padding: 12px 20px;
      border-radius: 8px;
      box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
      font-size: 14px;
      display: flex;
      align-items: center;
      gap: 10px;
      pointer-events: auto;
      animation: slideIn 0.3s ease-out;
    }

    @keyframes slideIn {
      from {
        transform: translateX(120%);
        opacity: 0;
      }
      to {
        transform: translateX(0);
        opacity: 1;
      }
    }

    .notification.fade-out {
      animation: fadeOut 0.3s ease-out forwards;
    }

    @keyframes fadeOut {
      to {
        opacity: 0;
        transform: translateY(10px);
      }
    }

    .spinner {
      width: 16px;
      height: 16px;
      border: 2px solid rgba(255, 255, 255, 0.3);
      border-top-color: white;
      border-radius: 50%;
      animation: spin 0.6s linear infinite;
    }

    @keyframes spin {
      to {
        transform: rotate(360deg);
      }
    }

    .status-dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: #4caf50;
    }

    .status-dot.disconnected {
      background: #f44336;
    }
  `;

  override connectedCallback() {
    super.connectedCallback();

    // Only enable live reload in server mode (not for static sites)
    const config = (window as any).__MBR_CONFIG__;
    if (!config?.serverMode) {
      console.log('[mbr-live-reload] Disabled (static mode)');
      return;
    }

    // Get the current markdown file from frontmatter or URL
    this._currentMarkdownFile = this._detectCurrentMarkdownFile();

    // Connect to WebSocket
    this._connect();

    // Cleanup on page unload
    window.addEventListener('beforeunload', () => this._disconnect());
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    this._disconnect();
  }

  private _detectCurrentMarkdownFile(): string | null {
    // Try to get from window.frontmatter (injected by server)
    const frontmatter = (window as any).frontmatter;
    if (frontmatter?.markdown_source) {
      return frontmatter.markdown_source;
    }

    // Fallback: try to infer from URL path
    const path = window.location.pathname;
    if (path === '/') {
      return '/index.md';
    }

    // Convert URL path to potential markdown file path
    // e.g., /docs/guide/ -> docs/guide.md or docs/guide/index.md
    return path.replace(/\/$/, '') + '.md';
  }

  private _connect() {
    try {
      // Use wss:// for https, ws:// for http
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const wsUrl = `${protocol}//${window.location.host}/.mbr/ws/changes`;

      console.log('[mbr-live-reload] Connecting to:', wsUrl);
      this._ws = new WebSocket(wsUrl);

      this._ws.onopen = () => {
        console.log('[mbr-live-reload] WebSocket connected');
        this._reconnectAttempts = 0;
        this._reconnectDelay = 1000;
      };

      this._ws.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);

          // Handle connection status
          if (data.status === 'connected') {
            console.log('[mbr-live-reload] Connection confirmed');
            return;
          }

          // Handle error messages
          if (data.error) {
            console.error('[mbr-live-reload] Server error:', data.error);
            this._disconnect();
            return;
          }

          // Handle file change events
          if (data.relative_path) {
            this._handleFileChange(data as FileChangeEvent);
          }
        } catch (err) {
          console.error('[mbr-live-reload] Failed to parse message:', err);
        }
      };

      this._ws.onerror = (error) => {
        console.error('[mbr-live-reload] WebSocket error:', error);
      };

      this._ws.onclose = () => {
        console.log('[mbr-live-reload] WebSocket closed');
        this._attemptReconnect();
      };
    } catch (error) {
      console.error('[mbr-live-reload] Failed to create WebSocket:', error);
      this._attemptReconnect();
    }
  }

  private _disconnect() {
    if (this._ws) {
      this._ws.close();
      this._ws = null;
    }
  }

  private _attemptReconnect() {
    if (this._reconnectAttempts >= this._maxReconnectAttempts) {
      console.log('[mbr-live-reload] Max reconnection attempts reached. Giving up.');
      return;
    }

    this._reconnectAttempts++;
    const delay = this._reconnectDelay * Math.pow(1.5, this._reconnectAttempts - 1);

    console.log(`[mbr-live-reload] Reconnecting in ${delay}ms (attempt ${this._reconnectAttempts}/${this._maxReconnectAttempts})`);

    setTimeout(() => {
      this._connect();
    }, delay);
  }

  private _handleFileChange(event: FileChangeEvent) {
    console.log('[mbr-live-reload] File changed:', event);

    const changedPath = event.relative_path;
    const shouldReload = this._shouldReloadForFile(changedPath);

    if (shouldReload) {
      this._showReloadNotification(changedPath);

      // Reload after a short delay to show notification
      setTimeout(() => {
        window.location.reload();
      }, 500);
    }
  }

  private _shouldReloadForFile(changedPath: string): boolean {
    // Always reload for template and CSS changes
    if (changedPath.includes('.mbr/') &&
        (changedPath.endsWith('.html') ||
         changedPath.endsWith('.css') ||
         changedPath.endsWith('.js'))) {
      return true;
    }

    // Reload if the current markdown file changed
    if (this._currentMarkdownFile) {
      const normalizedCurrent = this._currentMarkdownFile.replace(/^\//, '');
      const normalizedChanged = changedPath.replace(/^\//, '');

      if (normalizedCurrent === normalizedChanged) {
        return true;
      }

      // Also check if changed file is index.md in current directory
      if (normalizedChanged.endsWith('/index.md') || normalizedChanged === 'index.md') {
        const currentDir = normalizedCurrent.split('/').slice(0, -1).join('/');
        const changedDir = normalizedChanged.split('/').slice(0, -1).join('/');

        if (currentDir === changedDir) {
          return true;
        }
      }
    }

    // Reload for any markdown file if we're on a directory listing
    if (!this._currentMarkdownFile || this._currentMarkdownFile.endsWith('/')) {
      if (changedPath.endsWith('.md')) {
        return true;
      }
    }

    return false;
  }

  private _showReloadNotification(_file: string) {
    this._showNotification = true;

    // Auto-hide after reload begins
    setTimeout(() => {
      this._showNotification = false;
    }, 2000);
  }

  override render() {
    if (!this._showNotification) {
      return html``;
    }

    return html`
      <div class="notification">
        <div class="spinner"></div>
        <span>Reloading page...</span>
      </div>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-live-reload': MbrLiveReloadElement;
  }
}
