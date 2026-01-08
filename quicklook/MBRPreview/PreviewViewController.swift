// PreviewViewController.swift
// MBR Markdown QuickLook Extension
//
// Renders markdown files using MBR's rendering engine via UniFFI bindings.

import Cocoa
import Quartz
import WebKit
import os.log

private let logger = OSLog(subsystem: "com.zmre.mbr.MBRPreview", category: "Preview")

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

        // Allow file:// URLs for local images and assets
        config.preferences.setValue(true, forKey: "allowFileAccessFromFileURLs")

        // Create WebView - QuickLook will resize it
        webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 600), configuration: config)
        webView.autoresizingMask = [.width, .height]
        webView.navigationDelegate = self

        // Set the webview directly as the view
        self.view = webView

        os_log(.error, log: logger, "loadView complete, webView is the view")
    }

    // MARK: - WKNavigationDelegate

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        os_log(.info, log: logger, "webView didFinish navigation")
        webView.needsDisplay = true
        completionHandler?(nil)
        completionHandler = nil
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        os_log(.error, log: logger, "webView didFail navigation: %{public}@", error.localizedDescription)
        completionHandler?(error)
        completionHandler = nil
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        os_log(.error, log: logger, "webView didFailProvisionalNavigation: %{public}@", error.localizedDescription)
        completionHandler?(error)
        completionHandler = nil
    }

    // MARK: - QLPreviewingController

    func preparePreviewOfFile(at url: URL, completionHandler handler: @escaping (Error?) -> Void) {
        os_log(.info, log: logger, "preparePreviewOfFile called for: %{public}@", url.path)

        // Store the completion handler - we'll call it when WebView finishes loading
        self.completionHandler = handler

        // Get the file path
        let filePath = url.path

        // Find config root by looking for .mbr/ directory
        let configRoot = findConfigRoot(for: url)
        os_log(.info, log: logger, "configRoot = %{public}@", configRoot ?? "nil")

        do {
            os_log(.info, log: logger, "calling renderPreview...")
            // Call Rust FFI to render markdown
            let html = try renderPreview(filePath: filePath, configRoot: configRoot)
            os_log(.info, log: logger, "renderPreview succeeded, HTML length = %d", html.count)

            // Load HTML in WebView with base URL for relative resources
            let baseURL = url.deletingLastPathComponent()
            os_log(.info, log: logger, "loading HTML with baseURL = %{public}@", baseURL.absoluteString)

            // Load the rendered HTML - completion handler will be called in didFinish delegate
            webView.loadHTMLString(html, baseURL: baseURL)

        } catch let error as QuickLookError {
            // Handle specific QuickLook errors
            os_log(.error, log: logger, "QuickLookError: %{public}@", error.localizedDescription)
            loadErrorHTML(message: error.localizedDescription)
            // For errors, we still load error HTML, so the handler will be called in didFinish
        } catch {
            // Handle unexpected errors
            os_log(.error, log: logger, "Unexpected error: %{public}@", error.localizedDescription)
            loadErrorHTML(message: error.localizedDescription)
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
    internal func findConfigRoot(for fileURL: URL) -> String? {
        var currentDir = fileURL.deletingLastPathComponent()
        let fileManager = FileManager.default

        // Search up to 10 levels deep
        for _ in 0..<10 {
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
    internal func loadErrorHTML(message: String) {
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

        webView.loadHTMLString(errorHTML, baseURL: nil)
    }
}
