import { describe, expect, it } from 'vitest';
import { heavyMediaKind, mediaProxyURL } from './editor-media-proxy.js';

describe('heavyMediaKind', () => {
  it('classifies representative video extensions', () => {
    for (const url of [
      'clip.mp4',
      'reel.mpg',
      'old.avi',
      'stream.ogv',
      'movie.m4v',
      'archive.mkv',
      'phone.mov',
    ]) {
      expect(heavyMediaKind(url)).toBe('video');
    }
  });

  it('classifies representative audio extensions', () => {
    for (const url of [
      'song.mp3',
      'take.wav',
      'master.flac',
      'clip.aac',
      'voice.m4a',
    ]) {
      expect(heavyMediaKind(url)).toBe('audio');
    }
  });

  it('classifies pdf', () => {
    expect(heavyMediaKind('report.pdf')).toBe('pdf');
  });

  it('resolves the .ogg → video and .webm → audio priority', () => {
    // Video is checked before audio, so `.ogg` (in both Rust sets) is video…
    expect(heavyMediaKind('sound.ogg')).toBe('video');
    // …while `.webm` (video-set absent) falls through to audio.
    expect(heavyMediaKind('sound.webm')).toBe('audio');
  });

  it('ignores a trailing ?query when parsing the extension', () => {
    expect(heavyMediaKind('/videos/x.mp4?token=abc123')).toBe('video');
    expect(heavyMediaKind('/audio/x.mp3?v=2')).toBe('audio');
    expect(heavyMediaKind('/docs/x.pdf?download=1')).toBe('pdf');
  });

  it('ignores a trailing #fragment when parsing the extension', () => {
    expect(heavyMediaKind('/videos/x.mp4#t=10,20')).toBe('video');
    expect(heavyMediaKind('/docs/x.pdf#page=3')).toBe('pdf');
  });

  it('is case-insensitive', () => {
    expect(heavyMediaKind('CLIP.MP4')).toBe('video');
    expect(heavyMediaKind('Song.Mp3')).toBe('audio');
    expect(heavyMediaKind('Report.PDF')).toBe('pdf');
  });

  it('returns null for real image extensions', () => {
    for (const url of ['a.png', 'a.jpg', 'a.jpeg', 'a.svg', 'a.gif', 'a.webp']) {
      expect(heavyMediaKind(url)).toBeNull();
    }
  });

  it('returns null for data: and blob: URLs', () => {
    expect(heavyMediaKind('data:image/png;base64,iVBORw0KGgoAAAANS')).toBeNull();
    expect(
      heavyMediaKind('data:image/svg+xml,%3Csvg%3E%3C/svg%3E'),
    ).toBeNull();
    expect(
      heavyMediaKind('blob:https://example.com/550e8400-e29b-41d4'),
    ).toBeNull();
  });

  it('returns null for extensionless and empty URLs', () => {
    expect(heavyMediaKind('https://example.com/watch')).toBeNull();
    expect(heavyMediaKind('/notes/no-extension')).toBeNull();
    expect(heavyMediaKind('')).toBeNull();
  });
});

describe('mediaProxyURL', () => {
  it('returns the original url unchanged for images and other pass-through URLs', () => {
    for (const url of [
      '/images/photo.png',
      'https://example.com/pic.jpg',
      'data:image/png;base64,iVBORw0KGgoAAAANS',
      'https://example.com/watch',
    ]) {
      expect(mediaProxyURL(url)).toBe(url);
    }
  });

  it('returns a data:image/svg+xml placeholder for each heavy-media kind', () => {
    for (const url of ['/videos/x.mp4', '/audio/x.mp3', '/docs/x.pdf']) {
      expect(mediaProxyURL(url)).toMatch(/^data:image\/svg\+xml,/);
    }
  });

  it('returns distinct placeholders for video, audio, and pdf', () => {
    const video = mediaProxyURL('x.mp4');
    const audio = mediaProxyURL('x.mp3');
    const pdf = mediaProxyURL('x.pdf');
    expect(new Set([video, audio, pdf]).size).toBe(3);
  });
});
