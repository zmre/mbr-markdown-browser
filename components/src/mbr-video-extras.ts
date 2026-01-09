import { LitElement, html, css, nothing, type CSSResultGroup } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

/**
 * Parsed chapter from VTT file.
 */
interface Chapter {
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
 * Parse a WebVTT file into chapters.
 */
function parseVtt(vttContent: string): Chapter[] {
  const chapters: Chapter[] = [];
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
        chapters.push({
          startTime,
          endTime,
          text: textLines.join(' '),
        });
      }
    } else {
      i++;
    }
  }

  return chapters;
}

/**
 * Video extras component that displays additional information about embedded videos.
 * Renders inside figcaption elements alongside video elements.
 *
 * Features:
 * - Displays start/end time range if provided
 * - Fetches chapters VTT file and displays current chapter during playback
 *
 * @attr src - The video source URL (required)
 * @attr start - Start time for playback (optional)
 * @attr end - End time for playback (optional)
 */
@customElement('mbr-video-extras')
export class MbrVideoExtrasElement extends LitElement {
  static override styles: CSSResultGroup = css`
    :host {
      display: inline;
      font-size: 0.85em;
      color: var(--pico-muted-color, #666);
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

  private _videoElement: HTMLVideoElement | null = null;
  private _chaptersTrack: TextTrack | null = null;
  private _chapters: Chapter[] = [];
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

    // Try to use the chapters track element first (browser handles VTT parsing)
    const chaptersTrack = this._findChaptersTrack(video);

    if (chaptersTrack) {
      // Ensure the track is showing so cues are available
      chaptersTrack.mode = 'hidden';
      this._chaptersTrack = chaptersTrack;

      // Wait for cues to load if not ready
      if (chaptersTrack.cues && chaptersTrack.cues.length > 0) {
        chaptersTrack.addEventListener('cuechange', this._boundCueChange);
        this._showInitialChapterFromTrack(chaptersTrack);
      } else {
        // Cues not loaded yet, wait for them
        const trackElement = video.querySelector('track[kind="chapters"]') as HTMLTrackElement | null;
        if (trackElement) {
          trackElement.addEventListener('load', () => {
            if (chaptersTrack.cues && chaptersTrack.cues.length > 0) {
              chaptersTrack.addEventListener('cuechange', this._boundCueChange);
              this._showInitialChapterFromTrack(chaptersTrack);
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

    // If we have manually parsed chapters, use timeupdate
    if (this._chapters.length > 0 && !this._chaptersTrack) {
      video.addEventListener('timeupdate', this._boundTimeUpdate);
      this._showInitialChapter();
    }
  }

  private _findChaptersTrack(video: HTMLVideoElement): TextTrack | null {
    for (let i = 0; i < video.textTracks.length; i++) {
      const track = video.textTracks[i];
      if (track.kind === 'chapters') {
        return track;
      }
    }
    return null;
  }

  private async _fetchChaptersManually() {
    if (!this.src) return;

    const chaptersUrl = `${this.src}.chapters.en.vtt`;

    try {
      const response = await fetch(chaptersUrl);
      if (!response.ok) return;

      const vttContent = await response.text();
      this._chapters = parseVtt(vttContent);

      if (this._chapters.length > 0 && this._videoElement) {
        this._videoElement.addEventListener('timeupdate', this._boundTimeUpdate);
        this._showInitialChapter();
      }
    } catch {
      // Chapters file doesn't exist or failed to load, that's OK
    }
  }

  /**
   * Get the initial time to show chapter for (start attribute or 0).
   */
  private _getInitialTime(): number {
    if (this.start && this.start.length > 0) {
      // Parse start time - could be seconds or HH:MM:SS format
      return parseVttTime(this.start);
    }
    return 0;
  }

  /**
   * Show initial chapter based on start time using parsed chapters.
   */
  private _showInitialChapter() {
    const time = this._getInitialTime();
    for (const chapter of this._chapters) {
      if (time >= chapter.startTime && time < chapter.endTime) {
        this._currentChapter = chapter.text;
        return;
      }
    }
  }

  /**
   * Show initial chapter based on start time using track cues.
   */
  private _showInitialChapterFromTrack(track: TextTrack) {
    if (!track.cues) return;

    const time = this._getInitialTime();
    for (let i = 0; i < track.cues.length; i++) {
      const cue = track.cues[i] as VTTCue;
      if (time >= cue.startTime && time < cue.endTime) {
        this._currentChapter = cue.text;
        return;
      }
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
    if (!this._videoElement || this._chapters.length === 0) return;

    const currentTime = this._videoElement.currentTime;
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

  override render() {
    const hasStart = this.start && this.start.length > 0;
    const hasEnd = this.end && this.end.length > 0;
    const hasTimeRange = hasStart || hasEnd;
    const hasChapter = this._currentChapter.length > 0;

    // If no time range and no chapter, render nothing
    if (!hasTimeRange && !hasChapter) {
      return nothing;
    }

    return html`
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
      ${hasChapter
        ? html`<span class="chapter"><span class="separator">&middot;</span>${this._currentChapter}</span>`
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
