// ThumbnailGeometry.swift
// MBR Markdown QuickLook Thumbnail Extension
//
// The page geometry and error types shared by ThumbnailProvider and
// WebSnapshotter, kept free of WebKit and QuickLook so tests reach them directly.

import CoreGraphics

/// Size and scale arithmetic for a thumbnail, kept free of WebKit and QuickLook
/// so it can be tested directly.
enum ThumbnailGeometry {
    /// The page the snapshot is laid out on, in CSS pixels: 800 wide, at the
    /// 8.5 x 11 aspect of a US Letter sheet.
    ///
    /// Fixed rather than taken from the request because the request's size is
    /// the size of the *icon*, often 64 or 128 points. Laid out that narrow,
    /// pico's responsive layout would show a phone-width page with a few words
    /// per line; laid out at 800 and scaled down, the thumbnail is a recognisable
    /// miniature of the page the preview shows.
    static let pageSize = CGSize(width: 800, height: 800 * 11 / 8.5)

    /// The largest size with the page's aspect that fits in `maximumSize`, in
    /// points, rounded down to whole points and never smaller than 1 x 1.
    ///
    /// QuickLook frames whatever it gets in the icon slot, so returning the
    /// page's own aspect (rather than filling the square it asked for) is what
    /// makes the thumbnail look like a sheet of paper and not a cropped one.
    static func contextSize(fitting maximumSize: CGSize) -> CGSize {
        guard maximumSize.width > 0, maximumSize.height > 0 else {
            return CGSize(width: 1, height: 1)
        }
        let scale = min(maximumSize.width / self.pageSize.width, maximumSize.height / self.pageSize.height)
        return CGSize(
            width: max(1, (self.pageSize.width * scale).rounded(.down)),
            height: max(1, (self.pageSize.height * scale).rounded(.down))
        )
    }
}

/// Errors from snapshotting a page. Every one ends in `handler(nil, error)`,
/// so the system draws its generic document icon.
enum SnapshotError: Error {
    /// The `^https?://` blocking rules could not be compiled. Fails closed:
    /// without them, a remote image in the note could stall the snapshot.
    case contentRulesUnavailable(Error?)
    /// WebKit returned no image and no error.
    case noImage
    /// The page did not paint within the deadline plus the snapshot grace
    /// period. The web view is discarded, since whatever hung it may still be
    /// running.
    case timedOut
    /// The page's WebContent process died.
    case webContentTerminated
}
