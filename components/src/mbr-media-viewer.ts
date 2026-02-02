import { LitElement, html, css, nothing, type CSSResultGroup, type TemplateResult } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

/**
 * Supported media types for the viewer.
 */
export type MediaType = 'video' | 'pdf' | 'audio' | 'image';

/**
 * Props for the media viewer component.
 */
export interface MediaViewerProps {
  /** Type of media to render */
  mediaType: MediaType;
}

/**
 * Media viewer component that renders video, PDF, or audio content.
 *
 * Reads the media path from URL query parameter (?path=...) and renders
 * the appropriate media element. Supports:
 * - Video: Native HTML5 video player with mbr-video-extras for chapters/transcripts
 * - PDF: Embedded PDF viewer using object/embed fallback
 * - Audio: Native HTML5 audio player with waveform visualization (future)
 * - Image: Native image display with responsive sizing
 *
 * @attr media-type - The type of media to render ('video', 'pdf', or 'audio')
 *
 * @example
 * ```html
 * <mbr-media-viewer media-type="video"></mbr-media-viewer>
 * ```
 */
@customElement('mbr-media-viewer')
export class MbrMediaViewerElement extends LitElement {
  static override styles: CSSResultGroup = css`
    :host {
      display: block;
      width: 100%;
    }

    .media-wrapper {
      width: 100%;
      max-width: 100%;
    }

    .error {
      padding: 1rem;
      background: var(--pico-del-background-color, #fdd);
      border: 1px solid var(--pico-del-color, #c00);
      border-radius: 4px;
      color: var(--pico-del-color, #c00);
    }

    .loading {
      padding: 2rem;
      text-align: center;
      color: var(--pico-muted-color, #666);
    }

    /* Video styles */
    video {
      width: 100%;
      max-height: 80vh;
      background: #000;
    }

    figure {
      margin: 0;
    }

    figcaption {
      margin-top: 0.5rem;
      padding: 0.5rem;
      background: var(--pico-card-background-color, #f9f9f9);
      border-radius: 4px;
    }

    /* PDF styles */
    .pdf-container {
      width: 100%;
      height: 80vh;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
    }

    object, embed {
      width: 100%;
      height: 100%;
    }

    .pdf-fallback {
      padding: 1rem;
      text-align: center;
    }

    .pdf-fallback a {
      color: var(--pico-primary, #1976d2);
    }

    /* Audio styles */
    .audio-wrapper {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 1rem;
      padding: 2rem;
      background: var(--pico-card-background-color, #f9f9f9);
      border-radius: 8px;
    }

    .audio-cover {
      max-width: 300px;
      max-height: 300px;
      border-radius: 8px;
      box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    }

    .audio-cover-placeholder {
      width: 200px;
      height: 200px;
      display: flex;
      align-items: center;
      justify-content: center;
      background: var(--pico-muted-border-color, #ccc);
      border-radius: 8px;
      color: var(--pico-muted-color, #666);
    }

    .audio-cover-placeholder svg {
      width: 64px;
      height: 64px;
      fill: currentColor;
    }

    audio {
      width: 100%;
      max-width: 400px;
    }

    .audio-info {
      margin-top: 0.5rem;
      font-size: 0.9em;
      color: var(--pico-muted-color, #666);
      text-align: center;
    }

    /* Image styles */
    .image-wrapper {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 1rem;
    }

    .image-wrapper img {
      max-width: 100%;
      max-height: 85vh;
      object-fit: contain;
      border-radius: 4px;
      box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
    }

    .image-info {
      font-size: 0.9em;
      color: var(--pico-muted-color, #666);
      text-align: center;
    }

    .no-path {
      padding: 2rem;
      text-align: center;
      color: var(--pico-muted-color, #666);
      background: var(--pico-card-background-color, #f9f9f9);
      border-radius: 4px;
    }
  `;

  /**
   * The type of media to render.
   */
  @property({ type: String, attribute: 'media-type' })
  mediaType: MediaType = 'video';

  /**
   * Resolved media path from URL query parameter.
   */
  @state()
  private _path: string | null = null;

  /**
   * Error message if path is invalid.
   */
  @state()
  private _error: string | null = null;

  /**
   * Loading state.
   */
  @state()
  private _loading = true;

  /**
   * Whether cover art exists for audio files.
   * Null means not checked, true/false after check.
   */
  @state()
  private _hasCoverArt: boolean | null = null;

  override connectedCallback(): void {
    super.connectedCallback();
    this._parseUrlPath();
  }

  /**
   * Parse the media path from URL query parameters.
   * Expected format: ?path=/videos/demo.mp4
   */
  private _parseUrlPath(): void {
    try {
      const params = new URLSearchParams(window.location.search);
      const path = params.get('path');

      if (!path) {
        this._error = null;
        this._path = null;
        this._loading = false;
        return;
      }

      // Basic path validation - prevent directory traversal
      if (path.includes('..')) {
        this._error = 'Invalid path: directory traversal not allowed';
        this._loading = false;
        return;
      }

      // Ensure path starts with /
      this._path = path.startsWith('/') ? path : '/' + path;
      this._error = null;
      this._loading = false;
    } catch (e) {
      this._error = 'Failed to parse media path';
      this._loading = false;
    }
  }

  /**
   * Get the poster URL for a video path.
   * Follows the sidecar pattern: video.mp4 -> video.mp4.cover.png
   */
  private _getPosterUrl(videoPath: string): string {
    return `${videoPath}.cover.png`;
  }

  /**
   * Render video content with native HTML5 player.
   * Includes poster image support for .cover.png sidecar files.
   */
  private _renderVideo(): TemplateResult {
    if (!this._path) return html``;

    const posterUrl = this._getPosterUrl(this._path);

    return html`
      <figure class="media-wrapper">
        <video
          controls
          preload="metadata"
          src="${this._path}"
          poster="${posterUrl}"
          @error="${this._handleVideoError}"
        >
          <p>Your browser does not support the video element.</p>
        </video>
        <figcaption>
          <mbr-video-extras src="${this._path}"></mbr-video-extras>
        </figcaption>
      </figure>
    `;
  }

  /**
   * Handle video load errors - typically just ignore poster failures
   * since they are optional and may not exist.
   */
  private _handleVideoError(event: Event): void {
    const video = event.target as HTMLVideoElement;
    // Log the error but don't break the UI
    if (video && !video.readyState) {
      console.warn('Video failed to load:', this._path);
    }
  }

  /**
   * Render PDF content with embedded viewer.
   */
  private _renderPdf(): TemplateResult {
    if (!this._path) return html``;

    return html`
      <div class="media-wrapper pdf-container">
        <object data="${this._path}" type="application/pdf">
          <embed src="${this._path}" type="application/pdf" />
          <div class="pdf-fallback">
            <p>Unable to display PDF inline.</p>
            <p><a href="${this._path}" target="_blank">Open PDF in new tab</a></p>
          </div>
        </object>
      </div>
    `;
  }

  /**
   * Get the cover art URL for an audio path.
   * Follows the sidecar pattern: audio.mp3 -> audio.mp3.cover.png
   */
  private _getCoverArtUrl(audioPath: string): string {
    return `${audioPath}.cover.png`;
  }

  /**
   * Handle cover art load error - hide the cover art element.
   */
  private _handleCoverArtError(): void {
    this._hasCoverArt = false;
  }

  /**
   * Handle cover art load success - show the cover art element.
   */
  private _handleCoverArtLoad(): void {
    this._hasCoverArt = true;
  }

  /**
   * Render a placeholder icon for audio without cover art.
   */
  private _renderAudioPlaceholder(): TemplateResult {
    return html`
      <div class="audio-cover-placeholder">
        <svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
          <path d="M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z"/>
        </svg>
      </div>
    `;
  }

  /**
   * Render audio content with native HTML5 player.
   * Includes cover art support for .cover.png sidecar files.
   */
  private _renderAudio(): TemplateResult {
    if (!this._path) return html``;

    // Extract filename from path for display
    const filename = this._path.split('/').pop() ?? 'Audio';
    const coverArtUrl = this._getCoverArtUrl(this._path);

    return html`
      <div class="media-wrapper audio-wrapper">
        ${this._hasCoverArt === false
          ? this._renderAudioPlaceholder()
          : html`
            <img
              class="audio-cover"
              src="${coverArtUrl}"
              alt="Album cover for ${filename}"
              @error="${this._handleCoverArtError}"
              @load="${this._handleCoverArtLoad}"
              style="${this._hasCoverArt === null ? 'display: none;' : ''}"
            />
            ${this._hasCoverArt === null ? this._renderAudioPlaceholder() : nothing}
          `}
        <audio controls preload="metadata" src="${this._path}">
          <p>Your browser does not support the audio element.</p>
        </audio>
        <div class="audio-info">
          <span>${filename}</span>
        </div>
      </div>
    `;
  }

  /**
   * Render image content with native img element.
   */
  private _renderImage(): TemplateResult {
    if (!this._path) return html``;

    // Extract filename from path for display
    const filename = this._path.split('/').pop() ?? 'Image';

    return html`
      <div class="media-wrapper image-wrapper">
        <img
          src="${this._path}"
          alt="${filename}"
          @error="${this._handleImageError}"
        />
        <div class="image-info">
          <span>${filename}</span>
        </div>
      </div>
    `;
  }

  /**
   * Handle image load errors.
   */
  private _handleImageError(): void {
    this._error = 'Failed to load image';
  }

  /**
   * Render content based on media type.
   */
  private _renderContent(): TemplateResult {
    switch (this.mediaType) {
      case 'video':
        return this._renderVideo();
      case 'pdf':
        return this._renderPdf();
      case 'audio':
        return this._renderAudio();
      case 'image':
        return this._renderImage();
      default:
        return html`<div class="error">Unknown media type: ${this.mediaType}</div>`;
    }
  }

  override render(): TemplateResult | typeof nothing {
    // Show loading state
    if (this._loading) {
      return html`<div class="loading">Loading...</div>`;
    }

    // Show error if present
    if (this._error) {
      return html`<div class="error">${this._error}</div>`;
    }

    // Show message if no path provided
    if (!this._path) {
      return html`
        <div class="no-path">
          <p>No media path specified.</p>
          <p>Add <code>?path=/path/to/media</code> to the URL to view media.</p>
        </div>
      `;
    }

    // Render the appropriate media content
    return this._renderContent();
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-media-viewer': MbrMediaViewerElement;
  }
}
