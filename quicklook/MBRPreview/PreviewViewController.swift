// PreviewViewController.swift
// MBR Markdown QuickLook Extension
//
// Renders markdown files using MBR's rendering engine via UniFFI bindings.

import Cocoa
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
class MBRFileSchemeHandler: NSObject, WKURLSchemeHandler {
    func webView(_: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
        NSLog("[MBRFileSchemeHandler] received request: %@", urlSchemeTask.request.url?.absoluteString ?? "nil")
        os_log(.info, log: logger, "MBRFileSchemeHandler received request: %{public}@", urlSchemeTask.request.url?.absoluteString ?? "nil")

        guard let url = urlSchemeTask.request.url,
              url.scheme == "mbrfile"
        else {
            os_log(.error, log: logger, "MBRFileSchemeHandler: invalid scheme in request")
            urlSchemeTask.didFailWithError(NSError(domain: "MBRPreview", code: -1, userInfo: [
                NSLocalizedDescriptionKey: "Invalid URL scheme",
            ]))
            return
        }

        // The path portion of mbrfile:///path/to/file is the actual file path
        let filePath = url.path
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
            os_log(.error, log: logger, "MBRFileSchemeHandler failed to read file: %{public}@ - %{public}@", filePath, error.localizedDescription)
            urlSchemeTask.didFailWithError(error)
        }
    }

    func webView(_: WKWebView, stop _: WKURLSchemeTask) {
        // No cleanup needed for synchronous file reads
    }

    /// Returns the MIME type for a file based on its extension.
    private func mimeType(for path: String) -> String {
        let ext = (path as NSString).pathExtension.lowercased()
        switch ext {
        // Video types
        case "mp4": return "video/mp4"
        case "webm": return "video/webm"
        case "mov": return "video/quicktime"
        case "m4v": return "video/x-m4v"
        case "ogv": return "video/ogg"

        // Image types
        case "png": return "image/png"
        case "jpg", "jpeg": return "image/jpeg"
        case "gif": return "image/gif"
        case "webp": return "image/webp"
        case "svg": return "image/svg+xml"
        case "ico": return "image/x-icon"
        case "bmp": return "image/bmp"
        case "tiff", "tif": return "image/tiff"
        case "heic", "heif": return "image/heic"

        // Document types
        case "pdf": return "application/pdf"

        // Web types
        case "html", "htm": return "text/html"
        case "css": return "text/css"
        case "js": return "application/javascript"
        case "json": return "application/json"
        case "xml": return "application/xml"

        // Font types
        case "woff": return "font/woff"
        case "woff2": return "font/woff2"
        case "ttf": return "font/ttf"
        case "otf": return "font/otf"

        default: return "application/octet-stream"
        }
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

    override func loadView() {
        os_log(.error, log: logger, "loadView called")

        // Create WebView configuration
        let config = WKWebViewConfiguration()
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")

        // Register custom URL scheme handler for local file access
        // The Rust side converts root-relative URLs (/videos/...) to mbrfile:// URLs
        // which this handler intercepts and serves from disk
        config.setURLSchemeHandler(MBRFileSchemeHandler(), forURLScheme: "mbrfile")

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

        // Find config root by looking for .mbr/ directory
        let configRoot = self.findConfigRoot(for: url)
        os_log(.info, log: logger, "configRoot = %{public}@", configRoot ?? "nil")

        do {
            os_log(.info, log: logger, "calling renderPreview...")
            // Call Rust FFI to render markdown
            let html = try renderPreview(filePath: filePath, configRoot: configRoot)
            os_log(.info, log: logger, "renderPreview succeeded, HTML length = %d", html.count)

            // Write HTML to debug file for inspection
            let debugPath = "/tmp/mbr-quicklook-debug.html"
            try? html.write(toFile: debugPath, atomically: true, encoding: .utf8)

            // Check if mbrfile:// URLs are present (for debugging)
            if html.contains("mbrfile://") {
                try? "HTML CONTAINS mbrfile:// URLs\n".write(toFile: "/tmp/mbr-quicklook-status.txt", atomically: true, encoding: .utf8)
                NSLog("[MBRPreview] HTML contains mbrfile:// URLs - scheme handler should intercept")
                os_log(.info, log: logger, "HTML contains mbrfile:// URLs - scheme handler should intercept")
                // Find a sample mbrfile URL for logging
                if let range = html.range(of: "mbrfile://[^'\"\\s]+", options: .regularExpression) {
                    let sample = String(html[range])
                    try? "Sample URL: \(sample)\n".write(toFile: "/tmp/mbr-quicklook-sample-url.txt", atomically: true, encoding: .utf8)
                    NSLog("[MBRPreview] Sample mbrfile URL: %@", sample)
                    os_log(.info, log: logger, "Sample mbrfile URL: %{public}@", sample)
                }
            } else {
                try? "HTML does NOT contain mbrfile:// URLs\n".write(toFile: "/tmp/mbr-quicklook-status.txt", atomically: true, encoding: .utf8)
                NSLog("[MBRPreview] HTML does NOT contain mbrfile:// URLs")
                os_log(.error, log: logger, "HTML does NOT contain mbrfile:// URLs - check Rust conversion")
            }

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

    /// Searches upward from the file location to find an `.mbr/` configuration directory.
    ///
    /// This method walks up the directory tree from the markdown file's location,
    /// checking each parent directory for an `.mbr/` subdirectory. The search is
    /// limited to 10 levels to prevent excessive filesystem traversal.
    ///
    /// - Parameter fileURL: The URL of the markdown file being previewed.
    /// - Returns: The path to the directory containing `.mbr/`, or `nil` if not found.
    func findConfigRoot(for fileURL: URL) -> String? {
        var currentDir = fileURL.deletingLastPathComponent()
        let fileManager = FileManager.default

        // Search up to 10 levels deep
        for _ in 0 ..< 10 {
            let mbrDir = currentDir.appendingPathComponent(".mbr")
            var isDirectory: ObjCBool = false

            if fileManager.fileExists(atPath: mbrDir.path, isDirectory: &isDirectory),
               isDirectory.boolValue {
                return currentDir.path
            }

            let parent = currentDir.deletingLastPathComponent()
            if parent == currentDir {
                // Reached filesystem root
                break
            }
            currentDir = parent
        }

        return nil
    }

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
