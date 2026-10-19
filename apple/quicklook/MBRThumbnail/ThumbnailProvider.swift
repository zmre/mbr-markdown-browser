// ThumbnailProvider.swift
// MBR Markdown QuickLook Thumbnail Extension
//
// Draws Finder thumbnails (icon and gallery view, the column-view preview pane,
// Get Info) for markdown files: the note's cover image when its frontmatter
// names one, else a snapshot of the same page the preview extension shows.

import AppKit
import Foundation
import os.log
import QuickLookThumbnailing

let thumbnailLog = OSLog(subsystem: "com.zmre.mbr.MBRThumbnail", category: "Thumbnail")

/// QuickLook thumbnail provider for markdown files.
///
/// # Why a second extension
///
/// One app extension serves one extension point. `MBRPreview.appex` is a
/// `com.apple.quicklook.preview` extension, which answers the spacebar panel
/// and nothing else; every *thumbnail* - Finder's icon and gallery views, the
/// column-view preview pane, Get Info - is asked of
/// `com.apple.quicklook.thumbnail`. With no provider there, macOS falls back
/// to its plain-text thumbnail, so those views showed raw markdown.
///
/// # Why WebKit
///
/// The page is drawn by the same HTML, theme and assets as the preview
/// (`render_thumbnail` in `src/quicklook.rs` builds it with the preview's own
/// code), so a thumbnail is a miniature of what pressing space shows. The cost
/// is a WebContent process, which ``WebSnapshotter`` keeps warm in a pool of
/// two. `os_log` records the duration of every thumbnail under the
/// `com.zmre.mbr.MBRThumbnail` subsystem so that cost stays measurable.
///
/// # Failure
///
/// Any failure answers `handler(nil, error)`, which makes the system draw its
/// generic document icon. Unlike the preview, which shows an error page rather
/// than fall back to plain text, a thumbnail must never be an error message:
/// it would be cached and shown at icon size, where it can only look like a
/// broken document.
@objc(ThumbnailProvider)
final class ThumbnailProvider: QLThumbnailProvider {
    override func provideThumbnail(
        for request: QLFileThumbnailRequest,
        _ handler: @escaping (QLThumbnailReply?, Error?) -> Void
    ) {
        let start = ContinuousClock.now
        let fileURL = request.fileURL
        let maximumSize = request.maximumSize
        let scale = request.scale

        // Rust first, on QuickLook's queue: it is pure CPU and file reads, and
        // the main thread belongs to the web views.
        let document: ThumbnailDocument
        do {
            let configRoot = findConfigRoot(filePath: fileURL.path)
            document = try renderThumbnail(filePath: fileURL.path, configRoot: configRoot)
        } catch {
            Self.finish(handler, .failure(error), fileURL: fileURL, kind: "render", since: start)
            return
        }

        switch document {
        case let .image(path):
            let reply = Self.reply(coverImageAt: path)
            Self.finish(handler, .success(reply), fileURL: fileURL, kind: "cover", since: start)

        case let .page(page):
            let contextSize = ThumbnailGeometry.contextSize(fitting: maximumSize)
            Task { @MainActor in
                do {
                    let image = try await WebSnapshotter.shared.snapshot(
                        page,
                        pixelWidth: contextSize.width * scale
                    )
                    let reply = Self.reply(drawing: image, contextSize: contextSize)
                    Self.finish(handler, .success(reply), fileURL: fileURL, kind: "page", since: start)
                } catch {
                    Self.finish(handler, .failure(error), fileURL: fileURL, kind: "page", since: start)
                }
            }
        }
    }

    /// The file-type badge on every thumbnail this extension answers.
    ///
    /// `extensionBadge` is QuickLook's own convention for this: the system
    /// draws the badge in its standard style, sized for the icon, so it stays
    /// readable at small sizes and matches other file types. Nothing is baked
    /// into the image, so the snapshot stays identical to the preview, and
    /// larger renderings that show no badge are untouched.
    static let extensionBadge = "MD"

    /// A reply for a note's cover image.
    ///
    /// The path is already proven to be a local image file inside the
    /// repository; QuickLook decodes and scales it itself.
    static func reply(coverImageAt path: String) -> QLThumbnailReply {
        let reply = QLThumbnailReply(imageFileURL: URL(fileURLWithPath: path))
        reply.extensionBadge = Self.extensionBadge
        return reply
    }

    /// A reply that draws `image` to fill the whole thumbnail.
    ///
    /// The Core Graphics variant (`drawing:`), not `currentContextDrawing:`,
    /// because it hands over the context itself, whose extent can be measured -
    /// see ``drawingRect(in:fallback:)`` for why that matters.
    static func reply(drawing image: NSImage, contextSize: CGSize) -> QLThumbnailReply {
        let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil)
        let reply = QLThumbnailReply(contextSize: contextSize) { context in
            guard let cgImage else {
                return false
            }
            context.interpolationQuality = .high
            context.draw(cgImage, in: Self.drawingRect(in: context, fallback: contextSize))
            return true
        }
        reply.extensionBadge = Self.extensionBadge
        return reply
    }

    /// The rectangle, in the context's user space, that covers all of it.
    ///
    /// `contextSize` is in points, but the context QuickLook passes is a bitmap
    /// of `contextSize x request.scale` *pixels* and, on macOS 26/27, carries
    /// no scale transform. Drawing into `CGRect(origin: .zero, size:
    /// contextSize)` therefore filled only the bottom-left quarter at 2x (and
    /// a ninth at 3x); every thumbnail in Finder showed it. So the extent is
    /// measured instead of assumed: the bitmap's pixel bounds mapped back
    /// through the inverse CTM is right whether or not a scale transform is
    /// applied. A context that is not a bitmap (no pixel size) falls back to
    /// its clip box, and one with no usable clip to `fallback`, in points.
    static func drawingRect(in context: CGContext, fallback: CGSize) -> CGRect {
        if context.width > 0, context.height > 0 {
            let device = CGRect(x: 0, y: 0, width: context.width, height: context.height)
            let user = device.applying(context.ctm.inverted())
            if user.width.isFinite, user.height.isFinite, user.width > 0, user.height > 0 {
                return user
            }
        }
        let clip = context.boundingBoxOfClipPath
        if !clip.isNull, !clip.isInfinite, clip.width > 0, clip.height > 0 {
            return clip
        }
        return CGRect(origin: .zero, size: fallback)
    }

    /// Answer QuickLook and log how long it took.
    private static func finish(
        _ handler: (QLThumbnailReply?, Error?) -> Void,
        _ outcome: Result<QLThumbnailReply, Error>,
        fileURL: URL,
        kind: StaticString,
        since start: ContinuousClock.Instant
    ) {
        let milliseconds = (ContinuousClock.now - start) / .milliseconds(1)
        switch outcome {
        case let .success(reply):
            os_log(
                .info,
                log: thumbnailLog,
                "thumbnail (%{public}s) in %.0f ms for %{public}@",
                kind.description,
                milliseconds,
                fileURL.path
            )
            handler(reply, nil)
        case let .failure(error):
            os_log(
                .error,
                log: thumbnailLog,
                "thumbnail failed (%{public}s) after %.0f ms for %{public}@: %{public}@",
                kind.description,
                milliseconds,
                fileURL.path,
                String(describing: error)
            )
            handler(nil, error)
        }
    }
}
