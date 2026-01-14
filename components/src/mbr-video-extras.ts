import { LitElement, html, css, nothing, type CSSResultGroup } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

/**
 * Parsed cue from VTT file (used for both chapters and captions).
 */
interface VttCue {
  startTime: number;
  endTime: number;
  text: string;
}

/**
 * Parse a VTT timestamp (HH:MM:SS.mmm or MM:SS.mmm) to seconds.
 */
function parseVttTime(timeStr: string): number {
  const parts = timeStr.trim().split(':');
  if (parts.length === 3) {
    const [hours, minutes, seconds] = parts;
    return parseFloat(hours) * 3600 + parseFloat(minutes) * 60 + parseFloat(seconds);
  } else if (parts.length === 2) {
    const [minutes, seconds] = parts;
    return parseFloat(minutes) * 60 + parseFloat(seconds);
  }
  return parseFloat(timeStr);
}

/**
 * Format seconds as MM:SS or HH:MM:SS.
 */
function formatTime(seconds: number): string {
  const hrs = Math.floor(seconds / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  const secs = Math.floor(seconds % 60);

  if (hrs > 0) {
    return `${hrs}:${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
  }
  return `${mins}:${secs.toString().padStart(2, '0')}`;
}

/**
 * Parse a WebVTT file into cues.
 */
function parseVtt(vttContent: string): VttCue[] {
  const cues: VttCue[] = [];
  const lines = vttContent.split('\n');
  let i = 0;

  // Skip header
  while (i < lines.length && !lines[i].includes('-->')) {
    i++;
  }

  while (i < lines.length) {
    const line = lines[i].trim();

    // Look for timestamp line (00:00:00.000 --> 00:00:10.000)
    if (line.includes('-->')) {
      const [startStr, endStr] = line.split('-->').map(s => s.trim());
      const startTime = parseVttTime(startStr);
      const endTime = parseVttTime(endStr);

      // Collect text lines until empty line or end
      i++;
      const textLines: string[] = [];
      while (i < lines.length && lines[i].trim() !== '') {
        textLines.push(lines[i].trim());
        i++;
      }

      if (textLines.length > 0) {
        cues.push({
          startTime,
          endTime,
          text: textLines.join(' '),
        });
      }
    } else {
      i++;
    }
  }

  return cues;
}

/**
 * Video extras component that displays additional information about embedded videos.
 * Renders inside figcaption elements alongside video elements.
 *
 * Features:
 * - Displays start/end time range if provided
 * - Fetches chapters VTT file and displays current chapter during playback
 * - Clickable chapter opens modal with all chapters for navigation
 * - Toggle-able transcript panel with highlighted current caption
 *
 * @attr src - The video source URL (required)
 * @attr start - Start time for playback (optional)
 * @attr end - End time for playback (optional)
 */
@customElement('mbr-video-extras')
export class MbrVideoExtrasElement extends LitElement {
  static override styles: CSSResultGroup = css`
    :host {
      display: block;
      font-size: 0.85em;
      color: var(--pico-muted-color, #666);
    }

    .info-line {
      display: flex;
      align-items: center;
      flex-wrap: wrap;
      gap: 0.5em;
    }

    .time-range {
      font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
    }

    .separator {
      margin: 0 0.25em;
    }

    .chapter {
      font-style: italic;
    }

    .chapter-button {
      background: none;
      border: none;
      padding: 0;
      margin: 0;
      font: inherit;
      font-style: italic;
      color: inherit;
      cursor: pointer;
      text-decoration: underline;
      text-decoration-style: dotted;
      text-underline-offset: 2px;
    }

    .chapter-button:hover {
      color: var(--pico-primary, #1976d2);
    }

    .transcript-toggle {
      margin-left: auto;
      display: flex;
      align-items: center;
      gap: 0.5em;
    }

    .transcript-toggle label {
      display: flex;
      align-items: center;
      gap: 0.5em;
      cursor: pointer;
      font-size: 0.9em;
      margin: 0;
    }

    .transcript-toggle input[type="checkbox"] {
      margin: 0;
    }

    .transcript-box {
      position: relative;
      margin-top: 0.75em;
      height: 30vh;
      overflow-y: auto;
      overflow-x: hidden;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      padding: 0.75em;
      background: var(--pico-background-color, #fff);
      font-size: 0.95em;
      line-height: 1.6;
    }

    .caption {
      padding: 0.25em 0.5em;
      margin: 0.125em 0;
      border-radius: 3px;
      transition: background-color 0.2s ease;
      cursor: pointer;
    }

    .caption.active {
      background: var(--pico-secondary-background, #f0f0f0);
      color: var(--pico-secondary-inverse, #1a1a1a);
    }

    .caption.past {
      opacity: 0.6;
    }

    /* Modal styles using Pico CSS patterns */
    .modal-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.5);
      display: flex;
      align-items: center;
      justify-content: center;
      z-index: 10000;
    }

    .modal-dialog {
      background: var(--pico-background-color, #fff);
      border-radius: 8px;
      box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
      max-width: 500px;
      width: 90vw;
      max-height: 80vh;
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }

    .modal-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 1rem 1.25rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #ccc);
    }

    .modal-header h3 {
      margin: 0;
      font-size: 1.1rem;
      color: var(--pico-color, #333);
    }

    .modal-close {
      background: none;
      border: none;
      font-size: 1.5rem;
      line-height: 1;
      cursor: pointer;
      color: var(--pico-muted-color, #666);
      padding: 0;
      width: 2rem;
      height: 2rem;
      display: flex;
      align-items: center;
      justify-content: center;
      border-radius: 4px;
    }

    .modal-close:hover {
      background: var(--pico-secondary-background, #f0f0f0);
      color: var(--pico-color, #333);
    }

    .modal-body {
      padding: 0.5rem 0;
      overflow-y: auto;
      flex: 1;
    }

    .chapter-list {
      list-style: none;
      margin: 0;
      padding: 0;
    }

    .chapter-item {
      display: flex;
      align-items: center;
      gap: 1rem;
      padding: 0.75rem 1.25rem;
      cursor: pointer;
      transition: background-color 0.15s ease;
      border: none;
      background: none;
      width: 100%;
      text-align: left;
      font: inherit;
      color: inherit;
    }

    .chapter-item:hover {
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .chapter-item.active {
      background: var(--pico-secondary-background, #f0f0f0);
      color: var(--pico-secondary-inverse, #1a1a1a);
    }

    .chapter-time {
      font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
      font-size: 0.85em;
      color: var(--pico-muted-color, #666);
      min-width: 4em;
    }

    .chapter-title {
      flex: 1;
    }

    .chapter-item.active .chapter-time {
      color: inherit;
    }
  `;

  /**
   * The video source URL.
   */
  @property({ type: String })
  src = '';

  /**
   * Start time for video playback.
   */
  @property({ type: String })
  start = '';

  /**
   * End time for video playback.
   */
  @property({ type: String })
  end = '';

  /**
   * Current chapter text (reactive state).
   */
  @state()
  private _currentChapter = '';

  /**
   * Whether chapters modal is visible.
   */
  @state()
  private _showChaptersModal = false;

  /**
   * Whether transcript is visible.
   */
  @state()
  private _showTranscript = false;

  /**
   * Captions/transcript data.
   */
  @state()
  private _captions: VttCue[] = [];

  /**
   * Current caption index (-1 if none active).
   */
  @state()
  private _currentCaptionIndex = -1;

  /**
   * Whether captions have been loaded.
   */
  @state()
  private _captionsLoaded = false;

  /**
   * Parsed chapters data (reactive to enable clickable chapter button).
   */
  @state()
  private _chapters: VttCue[] = [];

  private _videoElement: HTMLVideoElement | null = null;
  private _chaptersTrack: TextTrack | null = null;
  private _boundTimeUpdate = this._onTimeUpdate.bind(this);
  private _boundCueChange = this._onCueChange.bind(this);

  override connectedCallback() {
    super.connectedCallback();
    // Defer setup to ensure DOM is ready
    requestAnimationFrame(() => this._setupVideoListener());
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    this._cleanup();
  }

  private _cleanup() {
    if (this._videoElement) {
      this._videoElement.removeEventListener('timeupdate', this._boundTimeUpdate);
    }
    if (this._chaptersTrack) {
      this._chaptersTrack.removeEventListener('cuechange', this._boundCueChange);
    }
    this._videoElement = null;
    this._chaptersTrack = null;
  }

  private async _setupVideoListener() {
    // Find the video element - it's a sibling of our parent (figcaption)
    const figcaption = this.parentElement;
    if (!figcaption) return;

    const figure = figcaption.parentElement;
    if (!figure) return;

    const video = figure.querySelector('video');
    if (!video) return;

    this._videoElement = video;

    // Always add timeupdate for captions tracking
    video.addEventListener('timeupdate', this._boundTimeUpdate);

    // Try to use the chapters track element first (browser handles VTT parsing)
    const chaptersTrack = this._findTrackByKind(video, 'chapters');

    if (chaptersTrack) {
      // Ensure the track is showing so cues are available
      chaptersTrack.mode = 'hidden';
      this._chaptersTrack = chaptersTrack;

      // Wait for cues to load if not ready
      if (chaptersTrack.cues && chaptersTrack.cues.length > 0) {
        chaptersTrack.addEventListener('cuechange', this._boundCueChange);
        this._extractChaptersFromTrack(chaptersTrack);
        this._showInitialChapter();
      } else {
        // Cues not loaded yet, wait for them
        const trackElement = video.querySelector('track[kind="chapters"]') as HTMLTrackElement | null;
        if (trackElement) {
          trackElement.addEventListener('load', () => {
            if (chaptersTrack.cues && chaptersTrack.cues.length > 0) {
              chaptersTrack.addEventListener('cuechange', this._boundCueChange);
              this._extractChaptersFromTrack(chaptersTrack);
              this._showInitialChapter();
            }
          }, { once: true });

          // Also try fetching manually as fallback
          trackElement.addEventListener('error', () => this._fetchChaptersManually(), { once: true });
        }
      }
    } else {
      // No track element, try fetching chapters manually
      await this._fetchChaptersManually();
    }

    // If we have manually parsed chapters, show initial
    if (this._chapters.length > 0) {
      this._showInitialChapter();
    }

    // Fetch captions for transcript
    await this._fetchCaptions();
  }

  private _findTrackByKind(video: HTMLVideoElement, kind: string): TextTrack | null {
    for (let i = 0; i < video.textTracks.length; i++) {
      const track = video.textTracks[i];
      if (track.kind === kind) {
        return track;
      }
    }
    return null;
  }

  /**
   * Extract chapters from a TextTrack into our VttCue format.
   */
  private _extractChaptersFromTrack(track: TextTrack) {
    if (!track.cues) return;

    this._chapters = [];
    for (let i = 0; i < track.cues.length; i++) {
      const cue = track.cues[i] as VTTCue;
      this._chapters.push({
        startTime: cue.startTime,
        endTime: cue.endTime,
        text: cue.text,
      });
    }
  }

  private async _fetchChaptersManually() {
    if (!this.src) return;

    const chaptersUrl = `${this.src}.chapters.en.vtt`;

    try {
      const response = await fetch(chaptersUrl);
      if (!response.ok) return;

      const vttContent = await response.text();
      this._chapters = parseVtt(vttContent);

      if (this._chapters.length > 0) {
        this._showInitialChapter();
      }
    } catch {
      // Chapters file doesn't exist or failed to load, that's OK
    }
  }

  private async _fetchCaptions() {
    if (!this.src) return;

    const captionsUrl = `${this.src}.captions.en.vtt`;

    try {
      const response = await fetch(captionsUrl);
      if (!response.ok) return;

      const vttContent = await response.text();
      this._captions = parseVtt(vttContent);
      this._captionsLoaded = true;

      // Set initial caption based on start time
      if (this._captions.length > 0) {
        this._updateCurrentCaption(this._getInitialTime());
      }
    } catch {
      // Captions file doesn't exist or failed to load
    }
  }

  /**
   * Get the initial time to show chapter/caption for (start attribute or 0).
   */
  private _getInitialTime(): number {
    if (this.start && this.start.length > 0) {
      // Parse start time - could be seconds or HH:MM:SS format
      return parseVttTime(this.start);
    }
    return 0;
  }

  /**
   * Show initial chapter based on start time.
   */
  private _showInitialChapter() {
    const time = this._getInitialTime();
    for (const chapter of this._chapters) {
      if (time >= chapter.startTime && time < chapter.endTime) {
        this._currentChapter = chapter.text;
        return;
      }
    }
    // If no chapter matches, show the first one if video starts at beginning
    if (time === 0 && this._chapters.length > 0) {
      this._currentChapter = this._chapters[0].text;
    }
  }

  private _onCueChange() {
    if (!this._chaptersTrack?.activeCues) return;

    if (this._chaptersTrack.activeCues.length > 0) {
      const cue = this._chaptersTrack.activeCues[0] as VTTCue;
      this._currentChapter = cue.text;
    } else {
      this._currentChapter = '';
    }
  }

  private _onTimeUpdate() {
    if (!this._videoElement) return;

    const currentTime = this._videoElement.currentTime;

    // Update chapter from manually parsed chapters (if not using track)
    if (this._chapters.length > 0 && !this._chaptersTrack) {
      let foundChapter = '';
      for (const chapter of this._chapters) {
        if (currentTime >= chapter.startTime && currentTime < chapter.endTime) {
          foundChapter = chapter.text;
          break;
        }
      }
      if (foundChapter !== this._currentChapter) {
        this._currentChapter = foundChapter;
      }
    }

    // Update current caption
    if (this._captions.length > 0) {
      this._updateCurrentCaption(currentTime);
    }
  }

  private _updateCurrentCaption(currentTime: number) {
    let newIndex = -1;

    for (let i = 0; i < this._captions.length; i++) {
      const caption = this._captions[i];
      if (currentTime >= caption.startTime && currentTime < caption.endTime) {
        newIndex = i;
        break;
      }
    }

    if (newIndex !== this._currentCaptionIndex) {
      this._currentCaptionIndex = newIndex;

      // Auto-scroll to current caption if transcript is visible
      if (this._showTranscript && newIndex >= 0) {
        this._scrollToCurrentCaption();
      }
    }
  }

  private _scrollToCurrentCaption() {
    // Use requestAnimationFrame to ensure DOM has updated
    requestAnimationFrame(() => {
      const transcriptBox = this.shadowRoot?.querySelector('.transcript-box') as HTMLElement | null;
      const activeCaption = this.shadowRoot?.querySelector('.caption.active') as HTMLElement | null;

      if (transcriptBox && activeCaption) {
        // Get positions relative to the transcript box
        const boxRect = transcriptBox.getBoundingClientRect();
        const captionRect = activeCaption.getBoundingClientRect();

        // Calculate where the caption is relative to the box's scroll position
        const captionTopInBox = captionRect.top - boxRect.top + transcriptBox.scrollTop;
        const captionHeight = captionRect.height;
        const boxHeight = boxRect.height;

        // Scroll so the active caption is centered in the box
        const targetScroll = captionTopInBox - (boxHeight / 2) + (captionHeight / 2);

        transcriptBox.scrollTop = Math.max(0, targetScroll);
      }
    });
  }

  private _onTranscriptToggle(e: Event) {
    const checkbox = e.target as HTMLInputElement;
    this._showTranscript = checkbox.checked;

    // Scroll to current caption when opening transcript
    if (this._showTranscript && this._currentCaptionIndex >= 0) {
      this._scrollToCurrentCaption();
    }
  }

  private _openChaptersModal() {
    if (this._chapters.length > 0) {
      this._showChaptersModal = true;
    }
  }

  private _closeChaptersModal() {
    this._showChaptersModal = false;
  }

  private _onModalBackdropClick(e: Event) {
    // Only close if clicking the backdrop itself, not the dialog
    if (e.target === e.currentTarget) {
      this._closeChaptersModal();
    }
  }

  private _jumpToChapter(chapter: VttCue) {
    if (this._videoElement) {
      this._videoElement.currentTime = chapter.startTime;
      this._currentChapter = chapter.text;
      // Start playing if not already
      if (this._videoElement.paused) {
        this._videoElement.play();
      }
    }
    this._closeChaptersModal();
  }

  private _jumpToCaption(caption: VttCue) {
    if (this._videoElement) {
      this._videoElement.currentTime = caption.startTime;
      // Start playing if paused
      if (this._videoElement.paused) {
        this._videoElement.play();
      }
    }
  }

  private _isCurrentChapter(chapter: VttCue): boolean {
    return chapter.text === this._currentChapter;
  }

  override render() {
    const hasStart = this.start && this.start.length > 0;
    const hasEnd = this.end && this.end.length > 0;
    const hasTimeRange = hasStart || hasEnd;
    const hasChapters = this._chapters.length > 0;
    const hasCurrentChapter = this._currentChapter.length > 0;
    const hasCaptions = this._captionsLoaded && this._captions.length > 0;

    // If nothing to show, render nothing
    if (!hasTimeRange && !hasCurrentChapter && !hasCaptions) {
      return nothing;
    }

    return html`
      <div class="info-line">
        ${hasTimeRange
        ? html`
              <span class="time-range">
                ${hasStart && hasEnd
            ? html`<span class="separator">&middot;</span>${this.start} &ndash; ${this.end}`
            : hasStart
              ? html`<span class="separator">&middot;</span>from ${this.start}`
              : html`<span class="separator">&middot;</span>to ${this.end}`
          }
              </span>
            `
        : nothing
      }
        ${hasCurrentChapter
        ? html`
              <span class="chapter">
                <span class="separator">&middot;</span>
                ${hasChapters
            ? html`<button class="chapter-button" @click=${this._openChaptersModal}>${this._currentChapter}</button>`
            : this._currentChapter
          }
              </span>
            `
        : nothing
      }
        ${hasCaptions
        ? html`
              <div class="transcript-toggle">
                <label>
                  <input
                    name="transcript"
                    type="checkbox"
                    role="switch"
                    .checked=${this._showTranscript}
                    @change=${this._onTranscriptToggle}
                  />
                  Show transcript
                </label>
              </div>
            `
        : nothing
      }
      </div>
      ${this._showTranscript && hasCaptions
        ? html`
            <div class="transcript-box">
              ${this._captions.map((caption, index) => html`
                <div
                  class="caption ${index === this._currentCaptionIndex ? 'active' : ''} ${index < this._currentCaptionIndex ? 'past' : ''}"
                  @click=${() => this._jumpToCaption(caption)}
                >
                  ${caption.text}
                </div>
              `)}
            </div>
          `
        : nothing
      }
      ${this._showChaptersModal && hasChapters
        ? html`
            <div class="modal-backdrop" @click=${this._onModalBackdropClick}>
              <div class="modal-dialog" role="dialog" aria-modal="true" aria-labelledby="chapters-title">
                <div class="modal-header">
                  <h3 id="chapters-title">Chapters</h3>
                  <button class="modal-close" @click=${this._closeChaptersModal} aria-label="Close">&times;</button>
                </div>
                <div class="modal-body">
                  <ul class="chapter-list">
                    ${this._chapters.map(chapter => html`
                      <li>
                        <button
                          class="chapter-item ${this._isCurrentChapter(chapter) ? 'active' : ''}"
                          @click=${() => this._jumpToChapter(chapter)}
                        >
                          <span class="chapter-time">${formatTime(chapter.startTime)}</span>
                          <span class="chapter-title">${chapter.text}</span>
                        </button>
                      </li>
                    `)}
                  </ul>
                </div>
              </div>
            </div>
          `
        : nothing
      }
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-video-extras': MbrVideoExtrasElement;
  }
}
