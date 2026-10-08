// PreviewProvider.swift
// MBR Markdown QuickLook Extension
//
// Renders markdown (and plain text / source) files using MBR's rendering engine
// via UniFFI bindings, and hands the result to QuickLook as a data-based reply.

import Foundation
import os.log
import QuickLookUI
import UniformTypeIdentifiers

private let logger = OSLog(subsystem: "com.zmre.mbr.MBRPreview", category: "Preview")

/// QuickLook preview provider for MBR-rendered files.
///
/// # Why this is data-based and not view-based
///
/// This extension used to be a `QLPreviewingController` view controller hosting
/// a `WKWebView`. Everything about that worked - the Rust render succeeded, the
/// web view finished its navigation, the completion handler fired - and macOS
/// threw the answer away anyway:
///
/// ```text
/// Error Domain=QuickLookPreviewErrors Code=1
/// "View based preview response received when expecting a generation based preview"
/// ```
///
/// macOS asks for a *generation-based* (data) preview. So the extension now
/// subclasses ``QLPreviewProvider`` (which is what `QLIsDataBasedPreview` in
/// `Info.plist` promises) and returns a ``QLPreviewReply`` carrying the HTML
/// directly. There is no web view left in this process, which also removes the
/// ViewBridge hosting and the completion-handler timing from the picture.
///
/// # Local assets
///
/// The old view-based path served sibling images, video and `.mbr/theme.css`
/// through a custom `mbrfile://` scheme handler registered on the web view. A
/// data-based reply has no scheme handler; local resources travel as
/// ``QLPreviewReply/attachments`` and the HTML addresses them as `cid:<id>`.
///
/// That is a *stronger* containment guarantee than the one it replaces, not a
/// weaker one. The scheme handler was reachable from the previewed document -
/// untrusted markdown may contain raw HTML, so a `<script>` in it could request
/// any `mbrfile://` path it liked, and the handler had to refuse everything
/// outside the repository itself. `cid:` resolves only against this dictionary,
/// which is built by `collect_preview_attachments` in `src/quicklook.rs` from
/// files it has already proven are regular files inside the previewed
/// repository. A path that never enters the dictionary is not addressable at
/// all.
@objc(PreviewProvider)
final class PreviewProvider: QLPreviewProvider, QLPreviewingController {
    /// Size hint for the preview panel. QuickLook shows loading UI at this size
    /// before the data arrives, and resizes to fit once it has.
    private static let contentSize = CGSize(width: 1000, height: 800)

    func providePreview(for request: QLFilePreviewRequest) async throws -> QLPreviewReply {
        let fileURL = request.fileURL
        os_log(.info, log: logger, "providePreview called for: %{public}@", fileURL.path)

        // Rendered here rather than inside the data creation block, which is
        // where `QLPreviewReply.h` says the heavy lifting belongs. The
        // attachments force it: see below.
        let rendered = Self.render(fileURL)

        let reply = QLPreviewReply(
            dataOfContentType: .html,
            contentSize: Self.contentSize
        ) { reply in
            reply.stringEncoding = .utf8
            return rendered.html
        }

        // MUST be set here, on the reply, and not inside the block above.
        //
        // `QLPreviewReply.h` says the reply passed into the data creation block
        // is "provided for convenience for any further updates to its
        // properties, such as attachments, during the data generation". On
        // macOS 26 that is not true: attachments assigned inside the block
        // never reach QuickLook. There is no error and no log line - every
        // `cid:` URL in the page simply resolves to nothing and every image is
        // blank. Measured both ways; this is the one that works.
        reply.attachments = rendered.attachments
        return reply
    }

    // MARK: - Private Helpers

    /// Render `fileURL` to the two halves of a reply.
    ///
    /// Never throws: a preview that fails is still a preview. A thrown error
    /// makes QuickLook fall back to the system plain-text previewer, which for a
    /// markdown file silently looks like "the extension is not installed" - the
    /// exact failure mode that made this bug so hard to see. An error page says
    /// what went wrong instead.
    private static func render(
        _ fileURL: URL
    ) -> (html: Data, attachments: [String: QLPreviewReplyAttachment]) {
        // Find config root by searching upward for repository markers. Uses the
        // Rust implementation via FFI so QuickLook and server mode agree.
        let configRoot = findConfigRoot(filePath: fileURL.path)
        os_log(.info, log: logger, "configRoot = %{public}@", configRoot)

        do {
            let document = try renderPreview(filePath: fileURL.path, configRoot: configRoot)
            let attachments = Self.loadAttachments(document.attachments)
            os_log(
                .info,
                log: logger,
                "rendered %d bytes of HTML with %d attachment(s)",
                document.html.count,
                attachments.count
            )
            return (Data(document.html.utf8), attachments)
        } catch {
            os_log(.error, log: logger, "render failed: %{public}@", error.localizedDescription)
            return (Data(Self.errorHTML(message: error.localizedDescription).utf8), [:])
        }
    }

    /// Read every attachment the renderer asked for.
    ///
    /// Each `path` is already absolute, canonical, and proven to be a regular
    /// file inside the previewed repository, so this only has to read it. A file
    /// that cannot be read (permissions, a race with a delete) is dropped: the
    /// page then shows a broken asset rather than no preview at all. The size
    /// budget lives in Rust, next to the code that decides what to attach.
    ///
    /// Internal rather than private so `PreviewProviderTests` can exercise it:
    /// `QLFilePreviewRequest` has no public initializer, so `providePreview` is
    /// not directly callable from a test.
    static func loadAttachments(
        _ attachments: [PreviewAttachment]
    ) -> [String: QLPreviewReplyAttachment] {
        var loaded: [String: QLPreviewReplyAttachment] = [:]
        for attachment in attachments {
            let url = URL(fileURLWithPath: attachment.path)
            // Read, not map: the attachment outlives this call and a file
            // truncated underneath a mapping faults the extension. Rust has
            // already capped the size, so a copy is affordable.
            guard let data = try? Data(contentsOf: url) else {
                os_log(
                    .error,
                    log: logger,
                    "could not read attachment: %{public}@",
                    attachment.path
                )
                continue
            }
            // Ask the system for the content type rather than carrying a MIME
            // table on either side of the FFI: it knows every type the machine
            // knows, and there is only one of it.
            let contentType = UTType(filenameExtension: url.pathExtension) ?? .data
            loaded[attachment.id] = QLPreviewReplyAttachment(data: data, contentType: contentType)
        }
        return loaded
    }

    /// A formatted error page for when rendering fails.
    ///
    /// The message is escaped: it can contain a file path, and a path can
    /// contain anything.
    static func errorHTML(message: String) -> String {
        let escapedMessage = message
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")

        return """
        <!DOCTYPE html>
        <html>
        <head>
            <meta charset="utf-8">
            <meta name="color-scheme" content="light dark">
            <style>
                body {
                    font-family: -apple-system, BlinkMacSystemFont, sans-serif;
                    padding: 40px;
                    background: #1a1a1a;
                    color: #e0e0e0;
                }
                .error {
                    background: #2d1f1f;
                    border: 1px solid #5c3c3c;
                    border-radius: 8px;
                    padding: 20px;
                }
                h1 { color: #ff6b6b; margin-top: 0; }
                pre {
                    background: #252525;
                    padding: 15px;
                    border-radius: 4px;
                    overflow-x: auto;
                    white-space: pre-wrap;
                    word-wrap: break-word;
                }
            </style>
        </head>
        <body>
            <div class="error">
                <h1>Preview Error</h1>
                <p>Failed to render markdown preview:</p>
                <pre>\(escapedMessage)</pre>
            </div>
        </body>
        </html>
        """
    }
}
