// PreviewViewController.swift
// MBR Markdown QuickLook Extension
//
// Renders markdown files using MBR's rendering engine via UniFFI bindings.

import Cocoa
import Darwin
import os.log
import Quartz
import WebKit

private let logger = OSLog(subsystem: "com.zmre.mbr.MBRPreview", category: "Preview")

// MARK: - MBRFileSchemeHandler

/// Handles mbrfile:// URLs by reading local files from disk.
///
/// This scheme handler allows the WebView to access local files without needing
/// to use `loadFileURL()` (which requires a temp file). The Rust side converts
/// root-relative URLs like `/videos/test.mp4` to `mbrfile:///path/to/root/videos/test.mp4`,
/// and this handler intercepts those requests and serves the file data.
///
/// # Security
///
/// The previewed markdown is untrusted: it can contain raw HTML, so a `<script>`
/// in it can request *any* mbrfile:// URL it likes, not just the ones the Rust
/// side generated. This handler is therefore the security boundary, and it obeys
/// one rule:
///
/// > Serve a file only if its fully resolved path lies inside `allowedRoot`.
///
/// `allowedRoot` is the previewed document's repository root and is set by
/// `PreviewViewController` before any HTML is loaded. Until it is set, every
/// request is refused, so a mistake in the setup path fails closed.
class MBRFileSchemeHandler: NSObject, WKURLSchemeHandler {
    /// Fully resolved path of the only directory this handler will read from.
    ///
    /// `nil` means "nothing is allowed yet". Read and written on the main thread
    /// only: `WKURLSchemeHandler` callbacks are delivered on the main thread, and
    /// so is `preparePreviewOfFile(at:completionHandler:)`.
    var allowedRoot: String?

    /// Fully resolved absolute path with every symbolic link and `..` expanded,
    /// or `nil` if the path does not exist.
    ///
    /// This is `realpath(3)` — the same primitive Rust's `Path::canonicalize`
    /// uses on the other side of the FFI, so both halves agree on what a path
    /// "really" is.
    static func realPath(_ path: String) -> String? {
        guard let resolved = realpath(path, nil) else { return nil }
        defer { free(resolved) }
        return String(cString: resolved)
    }

    /// Whether `path` lies strictly inside `root`. Both must already be resolved
    /// by `realPath`, otherwise a symlink or `..` could make this lie.
    ///
    /// The trailing separator is what keeps `/notes-evil/secret` from passing as
    /// inside `/notes`.
    static func isContained(_ path: String, in root: String) -> Bool {
        let prefix = root.hasSuffix("/") ? root : root + "/"
        return path.hasPrefix(prefix)
    }

    /// Fails the task with a message, without disclosing whether the target exists.
    private func refuse(_ urlSchemeTask: WKURLSchemeTask, reason: String) {
        os_log(.error, log: logger, "MBRFileSchemeHandler refused request: %{public}@", reason)
        urlSchemeTask.didFailWithError(NSError(domain: "MBRPreview", code: -1, userInfo: [
            NSLocalizedDescriptionKey: "Refused: \(reason)"
        ]))
    }

    func webView(_: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
        os_log(
            .info,
            log: logger,
            "MBRFileSchemeHandler received request: %{public}@",
            urlSchemeTask.request.url?.absoluteString ?? "nil"
        )

        guard let url = urlSchemeTask.request.url,
              url.scheme == "mbrfile"
        else {
            self.refuse(urlSchemeTask, reason: "invalid URL scheme")
            return
        }

        // The path portion of mbrfile:///path/to/file is the actual file path.
        // It is attacker-controlled, so nothing below may trust its spelling.
        guard let allowedRoot = self.allowedRoot else {
            self.refuse(urlSchemeTask, reason: "no preview root has been established")
            return
        }

        guard let filePath = Self.realPath(url.path),
              Self.isContained(filePath, in: allowedRoot)
        else {
            self.refuse(urlSchemeTask, reason: "path is outside the previewed repository")
            return
        }

        let fileURL = URL(fileURLWithPath: filePath)

        os_log(.info, log: logger, "MBRFileSchemeHandler loading file: %{public}@", filePath)

        do {
            let data = try Data(contentsOf: fileURL)
            let mimeType = self.mimeType(for: filePath)

            let response = URLResponse(
                url: url,
                mimeType: mimeType,
                expectedContentLength: data.count,
                textEncodingName: nil
            )

            urlSchemeTask.didReceive(response)
            urlSchemeTask.didReceive(data)
            urlSchemeTask.didFinish()
        } catch {
            os_log(
                .error,
                log: logger,
                "MBRFileSchemeHandler failed to read file: %{public}@ - %{public}@",
                filePath,
                error.localizedDescription
            )
            urlSchemeTask.didFailWithError(error)
        }
    }

    func webView(_: WKWebView, stop _: WKURLSchemeTask) {
        // No cleanup needed for synchronous file reads
    }

    /// MIME type mapping for common file extensions.
    private static let mimeTypes: [String: String] = [
        // Video types
        "mp4": "video/mp4",
        "webm": "video/webm",
        "mov": "video/quicktime",
        "m4v": "video/x-m4v",
        "ogv": "video/ogg",
        // Image types
        "png": "image/png",
        "jpg": "image/jpeg",
        "jpeg": "image/jpeg",
        "gif": "image/gif",
        "webp": "image/webp",
        "svg": "image/svg+xml",
        "ico": "image/x-icon",
        "bmp": "image/bmp",
        "tiff": "image/tiff",
        "tif": "image/tiff",
        "heic": "image/heic",
        "heif": "image/heic",
        // Document types
        "pdf": "application/pdf",
        // Web types
        "html": "text/html",
        "htm": "text/html",
        "css": "text/css",
        "js": "application/javascript",
        "json": "application/json",
        "xml": "application/xml",
        // Font types
        "woff": "font/woff",
        "woff2": "font/woff2",
        "ttf": "font/ttf",
        "otf": "font/otf"
    ]

    /// Returns the MIME type for a file based on its extension.
    private func mimeType(for path: String) -> String {
        let ext = (path as NSString).pathExtension.lowercased()
        return Self.mimeTypes[ext] ?? "application/octet-stream"
    }
}

/// QuickLook preview controller for rendering MBR markdown files.
///
/// This controller uses a WKWebView to display HTML rendered from markdown files
/// via the MBR Rust rendering engine through UniFFI bindings. It automatically
/// searches for `.mbr/` configuration directories to apply custom themes and settings.
///
/// The controller implements:
/// - `QLPreviewingController` for QuickLook integration
/// - `WKNavigationDelegate` for handling WebView load completion
@objc(PreviewViewController)
class PreviewViewController: NSViewController, QLPreviewingController, WKNavigationDelegate {
    private var webView: WKWebView!
    private var completionHandler: ((Error?) -> Void)?

    /// Retained so `preparePreviewOfFile` can confine it to the previewed
    /// document's repository before any HTML is loaded.
    let fileSchemeHandler = MBRFileSchemeHandler()

    override func loadView() {
        os_log(.error, log: logger, "loadView called")

        // Create WebView configuration
        let config = WKWebViewConfiguration()
        #if DEBUG
            // Never in a shipped extension: the previewed markdown is untrusted
            // and this exposes the Web Inspector to it.
            config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        #endif

        // Register custom URL scheme handler for local file access
        // The Rust side converts root-relative URLs (/videos/...) to mbrfile:// URLs
        // which this handler intercepts and serves from disk
        config.setURLSchemeHandler(self.fileSchemeHandler, forURLScheme: "mbrfile")

        // Create WebView - QuickLook will resize it
        self.webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 600), configuration: config)
        self.webView.autoresizingMask = [.width, .height]
        self.webView.navigationDelegate = self

        // Set the webview directly as the view
        self.view = self.webView

        // Request larger preview size (QuickLook may constrain based on available space)
        self.preferredContentSize = NSSize(width: 1000, height: 800)

        os_log(.error, log: logger, "loadView complete, webView is the view")
    }

    // MARK: - WKNavigationDelegate

    func webView(_ webView: WKWebView, didFinish _: WKNavigation!) {
        os_log(.info, log: logger, "webView didFinish navigation")
        webView.needsDisplay = true
        self.completionHandler?(nil)
        self.completionHandler = nil
    }

    func webView(_: WKWebView, didFail _: WKNavigation!, withError error: Error) {
        os_log(.error, log: logger, "webView didFail navigation: %{public}@", error.localizedDescription)
        self.completionHandler?(error)
        self.completionHandler = nil
    }

    func webView(_: WKWebView, didFailProvisionalNavigation _: WKNavigation!, withError error: Error) {
        os_log(.error, log: logger, "webView didFailProvisionalNavigation: %{public}@", error.localizedDescription)
        self.completionHandler?(error)
        self.completionHandler = nil
    }

    // MARK: - QLPreviewingController

    func preparePreviewOfFile(at url: URL, completionHandler handler: @escaping (Error?) -> Void) {
        os_log(.info, log: logger, "preparePreviewOfFile called for: %{public}@", url.path)

        // Store the completion handler - we'll call it when WebView finishes loading
        self.completionHandler = handler

        // Get the file path
        let filePath = url.path

        // Find config root by searching upward for repository markers
        // Uses the Rust implementation via FFI for consistent behavior with server mode
        let configRoot = findConfigRoot(filePath: filePath)
        os_log(.info, log: logger, "configRoot = %{public}@", configRoot)

        // Confine the WebView's file access to this document's repository.
        // Must happen before loadHTMLString: the WebView issues no mbrfile://
        // request until the HTML is loaded, and a nil root refuses them all.
        self.fileSchemeHandler.allowedRoot = MBRFileSchemeHandler.realPath(configRoot)
        if self.fileSchemeHandler.allowedRoot == nil {
            os_log(.error, log: logger, "could not resolve preview root; local assets will not load")
        }

        do {
            os_log(.info, log: logger, "calling renderPreview...")
            // Call Rust FFI to render markdown
            let html = try renderPreview(filePath: filePath, configRoot: configRoot)
            os_log(.info, log: logger, "renderPreview succeeded, HTML length = %d", html.count)

            // Debug-only: dumps the rendered document to a world-readable
            // location, so it must never run in a shipped extension.
            #if DEBUG
                try? html.write(
                    toFile: "/tmp/mbr-quicklook-debug.html",
                    atomically: true,
                    encoding: .utf8
                )
                if html.contains("mbrfile://") {
                    os_log(.info, log: logger, "HTML contains mbrfile:// URLs - scheme handler should intercept")
                    if let range = html.range(of: "mbrfile://[^'\"\\s]+", options: .regularExpression) {
                        os_log(.info, log: logger, "Sample mbrfile URL: %{public}@", String(html[range]))
                    }
                } else {
                    os_log(.error, log: logger, "HTML does NOT contain mbrfile:// URLs - check Rust conversion")
                }
            #endif

            // Load HTML in WebView
            // Note: The Rust code converts root-relative URLs (/path) to mbrfile:// URLs
            // which are handled by MBRFileSchemeHandler registered in loadView()
            self.webView.loadHTMLString(html, baseURL: nil)

        } catch let error as QuickLookError {
            // Handle specific QuickLook errors
            os_log(.error, log: logger, "QuickLookError: %{public}@", error.localizedDescription)
            loadErrorHTML(message: error.localizedDescription)
            // For errors, we still load error HTML, so the handler will be called in didFinish
        } catch {
            // Handle unexpected errors
            os_log(.error, log: logger, "Unexpected error: %{public}@", error.localizedDescription)
            self.loadErrorHTML(message: error.localizedDescription)
            // For errors, we still load error HTML, so the handler will be called in didFinish
        }
    }

    // MARK: - Private Helpers

    /// Loads a formatted error page in the WebView when markdown rendering fails.
    ///
    /// This method generates a styled HTML error page with the error message
    /// escaped to prevent XSS. The error page uses a dark theme consistent
    /// with macOS system appearance.
    ///
    /// - Parameter message: The error message to display to the user.
    func loadErrorHTML(message: String) {
        let escapedMessage = message
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")

        let errorHTML = """
        <!DOCTYPE html>
        <html>
        <head>
            <meta charset="utf-8">
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

        self.webView.loadHTMLString(errorHTML, baseURL: nil)
    }
}
