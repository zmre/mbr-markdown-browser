/**
 * Unit tests for types.ts helper functions.
 */
import { describe, it, expect } from 'vitest';
import {
  type OtherFileInfo,
  type StaticFileKind,
  isMediaKind,
  isMediaFile,
  getMediaType,
  getMediaTitle,
  getCoverImageUrl,
  getViewerUrl,
  getMediaTypeLabel,
  getFileExtension,
  formatFileSize,
  formatDuration,
  MEDIA_TYPE_PRIORITY,
} from './types.ts';

// Helper to create test file info
function createFileInfo(
  urlPath: string,
  kind: StaticFileKind
): OtherFileInfo {
  return {
    raw_path: urlPath.replace(/^\//, ''),
    url_path: urlPath,
    metadata: {
      path: urlPath.replace(/^\//, ''),
      kind,
    },
  };
}

describe('MEDIA_TYPE_PRIORITY', () => {
  it('has correct priority order: video > pdf > audio > image', () => {
    expect(MEDIA_TYPE_PRIORITY).toEqual(['video', 'pdf', 'audio', 'image']);
  });
});

describe('isMediaKind', () => {
  it('returns true for video kind', () => {
    const kind: StaticFileKind = { type: 'video' };
    expect(isMediaKind(kind)).toBe(true);
  });

  it('returns true for pdf kind', () => {
    const kind: StaticFileKind = { type: 'pdf' };
    expect(isMediaKind(kind)).toBe(true);
  });

  it('returns true for audio kind', () => {
    const kind: StaticFileKind = { type: 'audio' };
    expect(isMediaKind(kind)).toBe(true);
  });

  it('returns true for image kind', () => {
    const kind: StaticFileKind = { type: 'image' };
    expect(isMediaKind(kind)).toBe(true);
  });

  it('returns false for text kind', () => {
    const kind: StaticFileKind = { type: 'text' };
    expect(isMediaKind(kind)).toBe(false);
  });

  it('returns false for other kind', () => {
    const kind: StaticFileKind = { type: 'other' };
    expect(isMediaKind(kind)).toBe(false);
  });
});

describe('isMediaFile', () => {
  it('returns true for video file', () => {
    const file = createFileInfo('/videos/demo.mp4', { type: 'video' });
    expect(isMediaFile(file)).toBe(true);
  });

  it('returns false for text file', () => {
    const file = createFileInfo('/docs/readme.txt', { type: 'text' });
    expect(isMediaFile(file)).toBe(false);
  });
});

describe('getMediaType', () => {
  it('returns video for video kind', () => {
    const file = createFileInfo('/videos/demo.mp4', { type: 'video' });
    expect(getMediaType(file)).toBe('video');
  });

  it('returns pdf for pdf kind', () => {
    const file = createFileInfo('/docs/manual.pdf', { type: 'pdf' });
    expect(getMediaType(file)).toBe('pdf');
  });

  it('returns audio for audio kind', () => {
    const file = createFileInfo('/music/song.mp3', { type: 'audio' });
    expect(getMediaType(file)).toBe('audio');
  });

  it('returns image for image kind', () => {
    const file = createFileInfo('/images/photo.jpg', { type: 'image' });
    expect(getMediaType(file)).toBe('image');
  });

  it('returns undefined for text kind', () => {
    const file = createFileInfo('/docs/readme.txt', { type: 'text' });
    expect(getMediaType(file)).toBeUndefined();
  });

  it('returns undefined for other kind', () => {
    const file = createFileInfo('/files/data.bin', { type: 'other' });
    expect(getMediaType(file)).toBeUndefined();
  });
});

describe('getMediaTitle', () => {
  it('returns video title from metadata', () => {
    const file = createFileInfo('/videos/demo.mp4', {
      type: 'video',
      title: 'My Demo Video',
    });
    expect(getMediaTitle(file)).toBe('My Demo Video');
  });

  it('returns pdf title from metadata', () => {
    const file = createFileInfo('/docs/manual.pdf', {
      type: 'pdf',
      title: 'User Manual',
    });
    expect(getMediaTitle(file)).toBe('User Manual');
  });

  it('returns audio title from metadata', () => {
    const file = createFileInfo('/music/song.mp3', {
      type: 'audio',
      title: 'My Song',
    });
    expect(getMediaTitle(file)).toBe('My Song');
  });

  it('falls back to filename when no title in metadata', () => {
    const file = createFileInfo('/videos/demo.mp4', { type: 'video' });
    expect(getMediaTitle(file)).toBe('demo.mp4');
  });

  it('falls back to filename for image (no title field)', () => {
    const file = createFileInfo('/images/photo.jpg', { type: 'image' });
    expect(getMediaTitle(file)).toBe('photo.jpg');
  });

  it('handles path with multiple segments', () => {
    const file = createFileInfo('/nested/path/to/video.mp4', { type: 'video' });
    expect(getMediaTitle(file)).toBe('video.mp4');
  });

  it('returns full path as fallback when no filename', () => {
    const file = createFileInfo('/', { type: 'other' });
    expect(getMediaTitle(file)).toBe('/');
  });

  it('ignores empty title string', () => {
    const file = createFileInfo('/videos/demo.mp4', {
      type: 'video',
      title: '',
    });
    expect(getMediaTitle(file)).toBe('demo.mp4');
  });
});

describe('getCoverImageUrl', () => {
  it('returns image url as its own cover', () => {
    const file = createFileInfo('/images/photo.jpg', { type: 'image' });
    expect(getCoverImageUrl(file)).toBe('/images/photo.jpg');
  });

  it('returns sidecar cover for video', () => {
    const file = createFileInfo('/videos/demo.mp4', { type: 'video' });
    expect(getCoverImageUrl(file)).toBe('/videos/demo.mp4.cover.png');
  });

  it('returns sidecar cover for pdf', () => {
    const file = createFileInfo('/docs/manual.pdf', { type: 'pdf' });
    expect(getCoverImageUrl(file)).toBe('/docs/manual.pdf.cover.png');
  });

  it('returns sidecar cover for audio', () => {
    const file = createFileInfo('/music/song.mp3', { type: 'audio' });
    expect(getCoverImageUrl(file)).toBe('/music/song.mp3.cover.png');
  });

  it('returns null for text kind', () => {
    const file = createFileInfo('/docs/readme.txt', { type: 'text' });
    expect(getCoverImageUrl(file)).toBeNull();
  });

  it('returns null for other kind', () => {
    const file = createFileInfo('/files/data.bin', { type: 'other' });
    expect(getCoverImageUrl(file)).toBeNull();
  });
});

describe('getViewerUrl', () => {
  it('returns video viewer url with encoded path', () => {
    const file = createFileInfo('/videos/demo.mp4', { type: 'video' });
    expect(getViewerUrl(file)).toBe(
      '/.mbr/videos/?path=%2Fvideos%2Fdemo.mp4'
    );
  });

  it('returns pdf viewer url with encoded path', () => {
    const file = createFileInfo('/docs/manual.pdf', { type: 'pdf' });
    expect(getViewerUrl(file)).toBe('/.mbr/pdfs/?path=%2Fdocs%2Fmanual.pdf');
  });

  it('returns audio viewer url with encoded path', () => {
    const file = createFileInfo('/music/song.mp3', { type: 'audio' });
    expect(getViewerUrl(file)).toBe('/.mbr/audio/?path=%2Fmusic%2Fsong.mp3');
  });

  it('returns image viewer url for image', () => {
    const file = createFileInfo('/images/photo.jpg', { type: 'image' });
    expect(getViewerUrl(file)).toBe('/.mbr/images/?path=%2Fimages%2Fphoto.jpg');
  });

  it('returns direct url for text', () => {
    const file = createFileInfo('/docs/readme.txt', { type: 'text' });
    expect(getViewerUrl(file)).toBe('/docs/readme.txt');
  });

  it('returns direct url for other', () => {
    const file = createFileInfo('/files/data.bin', { type: 'other' });
    expect(getViewerUrl(file)).toBe('/files/data.bin');
  });

  it('encodes special characters in path', () => {
    const file = createFileInfo('/videos/my video (2024).mp4', {
      type: 'video',
    });
    expect(getViewerUrl(file)).toBe(
      '/.mbr/videos/?path=%2Fvideos%2Fmy%20video%20(2024).mp4'
    );
  });
});

describe('getMediaTypeLabel', () => {
  it('returns Video for video', () => {
    expect(getMediaTypeLabel('video')).toBe('Video');
  });

  it('returns PDF for pdf', () => {
    expect(getMediaTypeLabel('pdf')).toBe('PDF');
  });

  it('returns Audio for audio', () => {
    expect(getMediaTypeLabel('audio')).toBe('Audio');
  });

  it('returns Image for image', () => {
    expect(getMediaTypeLabel('image')).toBe('Image');
  });
});

describe('getFileExtension', () => {
  it('extracts mp4 extension', () => {
    expect(getFileExtension('/videos/demo.mp4')).toBe('mp4');
  });

  it('extracts PDF extension (lowercase)', () => {
    expect(getFileExtension('/docs/manual.PDF')).toBe('pdf');
  });

  it('handles multiple dots in filename', () => {
    expect(getFileExtension('/videos/demo.2024.mp4')).toBe('mp4');
  });

  it('returns empty for no extension', () => {
    expect(getFileExtension('/docs/README')).toBe('');
  });

  it('returns empty for trailing dot', () => {
    expect(getFileExtension('/docs/file.')).toBe('');
  });

  it('handles root path', () => {
    expect(getFileExtension('/')).toBe('');
  });

  it('handles empty string', () => {
    expect(getFileExtension('')).toBe('');
  });
});

describe('formatFileSize', () => {
  it('formats bytes', () => {
    expect(formatFileSize(100)).toBe('100 B');
  });

  it('formats kilobytes', () => {
    expect(formatFileSize(1024)).toBe('1.0 KB');
  });

  it('formats megabytes', () => {
    expect(formatFileSize(1024 * 1024)).toBe('1.0 MB');
  });

  it('formats gigabytes', () => {
    expect(formatFileSize(1024 * 1024 * 1024)).toBe('1.0 GB');
  });

  it('formats terabytes', () => {
    expect(formatFileSize(1024 * 1024 * 1024 * 1024)).toBe('1.0 TB');
  });

  it('shows one decimal place for KB+', () => {
    expect(formatFileSize(1536)).toBe('1.5 KB');
  });

  it('returns empty string for undefined', () => {
    expect(formatFileSize(undefined)).toBe('');
  });

  it('returns empty string for negative', () => {
    expect(formatFileSize(-100)).toBe('');
  });

  it('handles zero', () => {
    expect(formatFileSize(0)).toBe('0 B');
  });
});

describe('formatDuration', () => {
  it('passes through duration string', () => {
    expect(formatDuration('01:23:45')).toBe('01:23:45');
  });

  it('returns empty string for undefined', () => {
    expect(formatDuration(undefined)).toBe('');
  });

  it('returns empty string for empty string', () => {
    expect(formatDuration('')).toBe('');
  });
});
