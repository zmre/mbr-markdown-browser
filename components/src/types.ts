/**
 * Type definitions for the media browser component.
 *
 * These types mirror the Rust StaticFileKind enum serialization format
 * which uses `#[serde(tag = "type", rename_all = "lowercase")]`.
 *
 * @module types
 */

import { resolveUrl } from './shared.js';

// ============================================================================
// StaticFileKind discriminated union types
// ============================================================================

/**
 * PDF file with document metadata.
 * Mirrors Rust: StaticFileKind::Pdf
 */
export interface PdfKind {
  readonly type: 'pdf';
  readonly description?: string;
  readonly title?: string;
  readonly author?: string;
  readonly subject?: string;
  readonly num_pages?: number;
}

/**
 * Image file with dimensions.
 * Mirrors Rust: StaticFileKind::Image
 */
export interface ImageKind {
  readonly type: 'image';
  readonly width?: number;
  readonly height?: number;
}

/**
 * Video file with media metadata.
 * Mirrors Rust: StaticFileKind::Video
 */
export interface VideoKind {
  readonly type: 'video';
  readonly width?: number;
  readonly height?: number;
  readonly duration?: string;
  readonly title?: string;
  readonly genre?: string;
  readonly album?: string;
}

/**
 * Audio file with media metadata.
 * Mirrors Rust: StaticFileKind::Audio
 */
export interface AudioKind {
  readonly type: 'audio';
  readonly duration?: string;
  readonly title?: string;
}

/**
 * Plain text file (includes .srt, .vtt, .css, .txt, etc.)
 * Mirrors Rust: StaticFileKind::Text
 */
export interface TextKind {
  readonly type: 'text';
}

/**
 * Unknown or unsupported file type.
 * Mirrors Rust: StaticFileKind::Other (default)
 */
export interface OtherKind {
  readonly type: 'other';
}

/**
 * Discriminated union of all static file kinds.
 *
 * Use type narrowing with the `type` discriminant:
 * ```typescript
 * if (kind.type === 'video') {
 *   console.log(kind.duration);  // TypeScript knows duration exists
 * }
 * ```
 *
 * Mirrors Rust: StaticFileKind enum with serde(tag = "type")
 */
export type StaticFileKind =
  | PdfKind
  | ImageKind
  | VideoKind
  | AudioKind
  | TextKind
  | OtherKind;

// ============================================================================
// Media types
// ============================================================================

/**
 * Media types that the media browser displays.
 * Ordered by display priority (video first, image last).
 */
export type MediaType = 'video' | 'pdf' | 'audio' | 'image';

/**
 * Array of media types in priority order for tab display.
 * Video > PDF > Audio > Image
 */
export const MEDIA_TYPE_PRIORITY: readonly MediaType[] = [
  'video',
  'pdf',
  'audio',
  'image',
] as const;

// ============================================================================
// File metadata types
// ============================================================================

/**
 * Base metadata fields present on all static files.
 * Mirrors Rust: StaticFileMetadata struct
 *
 * - `path`: Filesystem path relative to repo root
 * - `created`: Unix timestamp (seconds since epoch)
 * - `modified`: Unix timestamp (seconds since epoch)
 * - `file_size_bytes`: File size in bytes
 * - `kind`: Discriminated union of file type metadata
 */
export interface StaticFileMetadata {
  readonly path: string;
  readonly created?: number;
  readonly modified?: number;
  readonly file_size_bytes?: number;
  readonly kind: StaticFileKind;
}

/**
 * Non-markdown file entry from site.json.
 * Represents static assets like videos, PDFs, images, audio.
 *
 * Mirrors Rust: OtherFileInfo struct
 */
export interface OtherFileInfo {
  /** URL path for serving (starts with /) */
  readonly url_path: string;
  /** File metadata including kind-specific fields */
  readonly metadata: StaticFileMetadata;
  /**
   * Extracted text content (for searchable file types).
   * Note: This field is skipped in Rust serialization (serde(skip)),
   * so it will not appear in site.json. Kept for type completeness.
   */
  readonly extracted_text?: string;
}

// ============================================================================
// Type guards
// ============================================================================

/**
 * Checks if a StaticFileKind is a displayable media type.
 *
 * Media types are: video, pdf, audio, image.
 * Non-media types are: text, other.
 *
 * @param kind - The file kind to check
 * @returns True if the kind is a media type (video, pdf, audio, or image)
 *
 * @example
 * ```typescript
 * if (isMediaKind(file.metadata.kind)) {
 *   // TypeScript knows kind is PdfKind | ImageKind | VideoKind | AudioKind
 *   displayInMediaBrowser(file);
 * }
 * ```
 */
export function isMediaKind(
  kind: StaticFileKind
): kind is PdfKind | ImageKind | VideoKind | AudioKind {
  return (
    kind.type === 'video' ||
    kind.type === 'pdf' ||
    kind.type === 'audio' ||
    kind.type === 'image'
  );
}

/**
 * Checks if a file is a displayable media file.
 *
 * @param file - The file info to check
 * @returns True if the file is a media type
 */
export function isMediaFile(file: OtherFileInfo): boolean {
  return isMediaKind(file.metadata.kind);
}

// ============================================================================
// Helper functions
// ============================================================================

/**
 * Get the media type from an OtherFileInfo entry.
 *
 * @param file - The file info to get media type from
 * @returns The media type, or undefined if not a displayable media type
 *
 * @example
 * ```typescript
 * const mediaType = getMediaType(file);
 * if (mediaType) {
 *   filterByType(mediaType);
 * }
 * ```
 */
export function getMediaType(file: OtherFileInfo): MediaType | undefined {
  const kind = file.metadata.kind;
  if (
    kind.type === 'video' ||
    kind.type === 'pdf' ||
    kind.type === 'audio' ||
    kind.type === 'image'
  ) {
    return kind.type;
  }
  return undefined;
}

/**
 * Get display title for a media file.
 *
 * Priority:
 * 1. Title from file metadata (video, PDF, audio)
 * 2. Filename extracted from URL path
 *
 * @param file - The file info to get title from
 * @returns The display title (never empty)
 *
 * @example
 * ```typescript
 * const title = getMediaTitle(file);
 * // Returns "My Video" if metadata.kind.title exists
 * // Otherwise returns "video.mp4" (filename from path)
 * ```
 */
export function getMediaTitle(file: OtherFileInfo): string {
  const kind = file.metadata.kind;

  // Check for title in metadata (videos, PDFs, and audio can have titles)
  if (kind.type === 'video' && kind.title) {
    return kind.title;
  }
  if (kind.type === 'pdf' && kind.title) {
    return kind.title;
  }
  if (kind.type === 'audio' && kind.title) {
    return kind.title;
  }

  // Fall back to filename from URL path
  const parts = file.url_path.split('/');
  const filename = parts[parts.length - 1];
  return filename || file.url_path;
}

/**
 * Get cover image URL for a media file.
 *
 * Cover image sources:
 * - Images: The image file itself is the cover
 * - Videos, PDFs, Audio: Sidecar `.cover.jpg` file (e.g., `/path/to/file.mp4.cover.jpg`)
 *
 * Note: The returned URL may not exist. The component should handle
 * fallback to CSS-based type indicators when the cover image fails to load.
 *
 * @param file - The file info to get cover image URL from
 * @returns The cover image URL, or null for non-media types
 *
 * @example
 * ```typescript
 * const coverUrl = getCoverImageUrl(file);
 * if (coverUrl) {
 *   // Try to load the cover image
 *   // On error, fall back to CSS gradient
 * }
 * ```
 */
export function getCoverImageUrl(file: OtherFileInfo): string | null {
  const kind = file.metadata.kind;

  if (kind.type === 'image') {
    // Images are their own cover
    return resolveUrl(file.url_path);
  }

  if (
    kind.type === 'video' ||
    kind.type === 'pdf' ||
    kind.type === 'audio'
  ) {
    // Sidecar cover image convention: /path/to/file.ext.cover.jpg
    return resolveUrl(`${file.url_path}.cover.jpg`);
  }

  return null;
}

/**
 * Get the viewer URL for a media file.
 *
 * Viewer URL patterns:
 * - Video: `/.mbr/videos/?path={encoded_url_path}`
 * - PDF: `/.mbr/pdfs/?path={encoded_url_path}`
 * - Audio: `/.mbr/audio/?path={encoded_url_path}`
 * - Image: `/.mbr/images/?path={encoded_url_path}`
 *
 * @param file - The file info to get viewer URL from
 * @returns The viewer URL (direct file URL for non-media types)
 *
 * @example
 * ```typescript
 * const viewerUrl = getViewerUrl(file);
 * // For video: "/.mbr/videos/?path=%2Fvideos%2Fdemo.mp4"
 * // For image: "/.mbr/images/?path=%2Fimages%2Fphoto.jpg"
 * window.location.href = viewerUrl;
 * ```
 */
export function getViewerUrl(file: OtherFileInfo): string {
  const kind = file.metadata.kind;

  switch (kind.type) {
    case 'video':
      return resolveUrl(`/.mbr/videos/?path=${encodeURIComponent(file.url_path)}`);
    case 'pdf':
      return resolveUrl(`/.mbr/pdfs/?path=${encodeURIComponent(file.url_path)}`);
    case 'audio':
      return resolveUrl(`/.mbr/audio/?path=${encodeURIComponent(file.url_path)}`);
    case 'image':
      return resolveUrl(`/.mbr/images/?path=${encodeURIComponent(file.url_path)}`);
    default:
      // Non-media types link directly to the file
      return resolveUrl(file.url_path);
  }
}

/**
 * Get human-readable label for a media type.
 *
 * @param type - The media type
 * @returns Human-readable label (e.g., "Video", "PDF", "Audio", "Image")
 */
export function getMediaTypeLabel(type: MediaType): string {
  switch (type) {
    case 'video':
      return 'Video';
    case 'pdf':
      return 'PDF';
    case 'audio':
      return 'Audio';
    case 'image':
      return 'Image';
  }
}

/**
 * Get the file extension from a URL path.
 *
 * @param urlPath - The URL path (e.g., "/videos/demo.mp4")
 * @returns The lowercase extension without dot (e.g., "mp4"), or empty string
 */
export function getFileExtension(urlPath: string): string {
  const filename = urlPath.split('/').pop() || '';
  const lastDot = filename.lastIndexOf('.');
  if (lastDot === -1 || lastDot === filename.length - 1) {
    return '';
  }
  return filename.slice(lastDot + 1).toLowerCase();
}

/**
 * Format file size in human-readable form.
 *
 * @param bytes - File size in bytes
 * @returns Formatted size (e.g., "1.5 MB", "256 KB", "1.2 GB")
 */
export function formatFileSize(bytes: number | null | undefined): string {
  if (bytes == null || bytes < 0) {
    return '';
  }

  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let unitIndex = 0;
  let size = bytes;

  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex++;
  }

  // Show 1 decimal place for KB and above, no decimals for bytes
  const precision = unitIndex > 0 ? 1 : 0;
  return `${size.toFixed(precision)} ${units[unitIndex]}`;
}

/**
 * Format duration string for display.
 *
 * The duration from Rust is already formatted as "HH:MM:SS" or "MM:SS".
 * This function passes it through, or returns empty string if undefined.
 *
 * @param duration - Duration string from metadata
 * @returns Formatted duration or empty string
 */
export function formatDuration(duration: string | undefined): string {
  return duration || '';
}
