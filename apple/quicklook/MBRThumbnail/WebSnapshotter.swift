// WebSnapshotter.swift
// MBR Markdown QuickLook Thumbnail Extension
//
// Draws a rendered preview document into an image by loading it into an
// offscreen WKWebView and snapshotting the top of the page.

// WebKit calls every delegate and scheme-handler method here on the main
// thread, but older SDKs (nixpkgs' 14.4, which the shipping build uses) do not
// annotate its protocols @MainActor. So those methods are `nonisolated` and
// enter the main actor with `MainActor.assumeIsolated`, which also traps if
// WebKit ever calls one elsewhere. Declaring them main-actor directly is a
// warning under Swift 5.10 ("cannot be used to satisfy nonisolated protocol
// requirement"). @preconcurrency quiets the Sendable warnings for WebKit types.

import AppKit
import os.log
import UniformTypeIdentifiers
@preconcurrency import WebKit

/// Serves `cid:` URLs from the attachment list of the page being snapshotted,
/// and from nothing else.
///
/// This is the thumbnail's counterpart of `QLPreviewReply.attachments`: the
/// HTML from `render_thumbnail` addresses local assets as `cid:<id>`, exactly as
/// a preview's does, and this handler answers from the list that came with it.
/// That list was built by `collect_preview_attachments` in `src/quicklook.rs`
/// from files it proved are regular files inside the note's repository, so a
/// `<script>` in the note can ask for `cid:anything` and get only those. A path
/// that never entered the list is not addressable.
///
/// Only images are served. A thumbnail shows images and nothing else a page
/// can reference locally; a video named by the page would otherwise be read in
/// full (up to Rust's 16 MiB per-asset cap) to paint one frame of one icon.
/// A video's `poster` image is an image and is still served.
@MainActor
final class AttachmentSchemeHandler: NSObject, WKURLSchemeHandler {
    static let scheme = "cid"

    /// Attachment id to absolute path, for the page currently loaded.
    var attachments: [String: String] = [:]

    /// Tasks started and not yet answered or stopped. A task leaves this map
    /// the moment WebKit stops it, because calling anything on a stopped task
    /// raises an Objective-C exception - which would take the whole extension
    /// down - and a read finishing after a new page replaced the old one is
    /// exactly that case. Only the key crosses to the reading thread.
    private var inFlight: [ObjectIdentifier: WKURLSchemeTask] = [:]

    /// The attachment id a `cid:` URL names, or `nil` for any other URL.
    static func attachmentID(for url: URL?) -> String? {
        guard let url, url.scheme?.lowercased() == scheme else {
            return nil
        }
        let id = String(url.absoluteString.dropFirst(self.scheme.count + 1))
        return id.isEmpty ? nil : id
    }

    /// The MIME type to serve `path` as, or `nil` if it is not an image.
    static func imageMIMEType(forPath path: String) -> String? {
        let pathExtension = (path as NSString).pathExtension
        guard let type = UTType(filenameExtension: pathExtension), type.conforms(to: .image) else {
            return nil
        }
        return type.preferredMIMEType ?? "application/octet-stream"
    }

    nonisolated func webView(_: WKWebView, start task: WKURLSchemeTask) {
        MainActor.assumeIsolated { self.start(task) }
    }

    nonisolated func webView(_: WKWebView, stop task: WKURLSchemeTask) {
        MainActor.assumeIsolated { self.inFlight[ObjectIdentifier(task)] = nil }
    }

    private func start(_ task: WKURLSchemeTask) {
        guard
            let url = task.request.url,
            let id = Self.attachmentID(for: url),
            let path = attachments[id],
            let mimeType = Self.imageMIMEType(forPath: path)
        else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }

        // Read off the main thread: an image can be megabytes, and the main
        // thread is the one every other thumbnail in flight is waiting on.
        let key = ObjectIdentifier(task)
        self.inFlight[key] = task
        DispatchQueue.global(qos: .userInitiated).async {
            let data = try? Data(contentsOf: URL(fileURLWithPath: path))
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    self.finish(key, url: url, mimeType: mimeType, data: data)
                }
            }
        }
    }

    private func finish(_ key: ObjectIdentifier, url: URL, mimeType: String, data: Data?) {
        guard let task = inFlight.removeValue(forKey: key) else {
            return
        }
        guard let data else {
            task.didFailWithError(URLError(.cannotOpenFile))
            return
        }
        task.didReceive(URLResponse(
            url: url,
            mimeType: mimeType,
            expectedContentLength: data.count,
            textEncodingName: nil
        ))
        task.didReceive(data)
        task.didFinish()
    }
}

/// One reusable offscreen web view and the per-page state it needs.
@MainActor
final class SnapshotView: NSObject, WKNavigationDelegate {
    let webView: WKWebView
    private let attachments = AttachmentSchemeHandler()

    /// Resumed by the navigation delegate when the current load ends.
    private var onLoadEnd: ((Result<Bool, Error>) -> Void)?
    private var navigation: WKNavigation?

    /// Set once the web view cannot be trusted with another page: its
    /// WebContent process died, or a snapshot never came back.
    private(set) var isBroken = false

    /// Makes every image load now rather than when it scrolls into view.
    ///
    /// mbr renders images `loading="lazy"`, and lazy loading waits for the image
    /// to be found visible - which, in a web view with no window, it never is.
    /// Every image stayed blank, the local ones included. Run at document end,
    /// before `load`, so the switched images are fetched in time to hold the
    /// `load` event like any eager image.
    static let eagerImagesScript = "for (const image of document.images) { image.loading = 'eager'; }"

    /// What "settled" means before a snapshot: web fonts swapped in (a theme's
    /// `@font-face`, or KaTeX's) and every image decoded. A failed image (a
    /// blocked remote one) counts as settled.
    static let settleScript = """
    await document.fonts.ready;
    await Promise.all(Array.from(document.images, (image) => image.decode().catch(() => {})));
    return true;
    """

    init(contentRules: WKContentRuleList) {
        let configuration = WKWebViewConfiguration()
        configuration.setURLSchemeHandler(self.attachments, forURLScheme: AttachmentSchemeHandler.scheme)
        configuration.userContentController.add(contentRules)
        configuration.userContentController.addUserScript(WKUserScript(
            source: Self.eagerImagesScript,
            injectionTime: .atDocumentEnd,
            forMainFrameOnly: true
        ))
        // Nothing a thumbnail loads should leave cookies, caches or local
        // storage behind - in the extension's container or anywhere else.
        configuration.websiteDataStore = .nonPersistent()
        configuration.mediaTypesRequiringUserActionForPlayback = .all

        self.webView = WKWebView(
            frame: CGRect(origin: .zero, size: ThumbnailGeometry.pageSize),
            configuration: configuration
        )
        // Always light. The system caches a thumbnail once, whatever the
        // appearance was when it was drawn, so a dark one drawn at night would
        // stay dark all day - and document thumbnails are paper-white by
        // convention. This also makes `prefers-color-scheme` light inside the
        // page, so the theme, pico and highlight.js all pick their light rules.
        self.webView.appearance = NSAppearance(named: .aqua)
        super.init()
        self.webView.navigationDelegate = self
    }

    /// Load `page`, then snapshot its top at `pixelWidth` pixels wide.
    ///
    /// The page gets until `deadline` to finish loading and settle its fonts;
    /// whatever has rendered by then is snapshotted. The snapshot itself gets
    /// `snapshotGrace` more. If even that does not come back, the WebContent
    /// process is assumed hung (a `while (true) {}` in the note would do it)
    /// and this view is marked broken so the pool replaces it.
    func snapshot(
        _ page: PreviewDocument,
        pixelWidth: CGFloat,
        deadline: ContinuousClock.Instant,
        snapshotGrace: Duration
    ) async throws -> NSImage {
        self.attachments.attachments = Dictionary(
            page.attachments.map { ($0.id, $0.path) },
            uniquingKeysWith: { first, _ in first }
        )

        let loaded = try await race(until: deadline) { resume in
            self.onLoadEnd = resume
            self.navigation = self.webView.loadHTMLString(page.html, baseURL: nil)
        } ?? false
        self.onLoadEnd = nil

        // Web fonts swap in after `load`; snapshotting before they do would
        // draw fallback glyphs.
        if loaded {
            _ = try await race(until: deadline) { resume in
                self.webView.callAsyncJavaScript(
                    Self.settleScript,
                    arguments: [:],
                    in: nil,
                    in: .page
                ) { _ in resume(.success(true)) }
            }
        }

        let image = try await self.takeSnapshot(pixelWidth: pixelWidth, grace: snapshotGrace)

        // `snapshotWidth` is in points and the pixels-per-point WebKit uses
        // offscreen is only a guess (there is no window). If the guess was
        // high the image comes back too small - a blurry thumbnail - so take
        // it again at the measured scale, which is then remembered. Once per
        // process: if WebKit caps the size, retaking every time would only
        // double the cost.
        let measured = Self.pixelWidth(of: image)
        guard Self.learnedScale == nil, measured > 0, measured < pixelWidth - 1 else {
            return image
        }
        let pointsAsked = pixelWidth / self.backingScaleFactor
        Self.learnedScale = measured / pointsAsked
        os_log(.info, log: thumbnailLog, "snapshot scale corrected to %.2f", Self.learnedScale ?? 0)
        return try await self.takeSnapshot(pixelWidth: pixelWidth, grace: snapshotGrace)
    }

    /// One `takeSnapshot` of the top of the page, `pixelWidth` pixels wide if
    /// ``backingScaleFactor`` is right, bounded by `grace`.
    private func takeSnapshot(pixelWidth: CGFloat, grace: Duration) async throws -> NSImage {
        let configuration = WKSnapshotConfiguration()
        configuration.rect = CGRect(origin: .zero, size: ThumbnailGeometry.pageSize)
        configuration.snapshotWidth = NSNumber(value: Double(pixelWidth / self.backingScaleFactor))

        let snapshot: NSImage?? = try await race(until: .now + grace) { resume in
            self.webView.takeSnapshot(with: configuration) { image, error in
                if let error {
                    resume(.failure(error))
                } else {
                    resume(.success(image))
                }
            }
        }
        switch snapshot {
        case let .some(.some(image)):
            return image
        case .some(.none):
            throw self.isBroken ? SnapshotError.webContentTerminated : SnapshotError.noImage
        case .none:
            self.isBroken = true
            throw SnapshotError.timedOut
        }
    }

    /// Pixels per point in the images `takeSnapshot` has returned, once one
    /// came back at a different scale than ``backingScaleFactor`` guessed.
    private static var learnedScale: CGFloat?

    /// Width in pixels of the largest bitmap behind `image`.
    static func pixelWidth(of image: NSImage) -> CGFloat {
        CGFloat(image.representations.map(\.pixelsWide).max() ?? 0)
    }

    /// The scale WebKit snapshots at: `snapshotWidth` is in points, and the
    /// image it returns has this many pixels per point. An offscreen view has
    /// no window to take it from, so this is the screen's - unless a snapshot
    /// has already shown otherwise.
    private var backingScaleFactor: CGFloat {
        Self.learnedScale ?? self.webView.window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 1
    }

    // MARK: - WKNavigationDelegate

    nonisolated func webView(_: WKWebView, didFinish navigation: WKNavigation?) {
        MainActor.assumeIsolated { self.endLoad(navigation, .success(true)) }
    }

    nonisolated func webView(_: WKWebView, didFail navigation: WKNavigation?, withError error: Error) {
        MainActor.assumeIsolated { self.endLoad(navigation, .failure(error)) }
    }

    nonisolated func webView(
        _: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation?,
        withError error: Error
    ) {
        MainActor.assumeIsolated { self.endLoad(navigation, .failure(error)) }
    }

    nonisolated func webViewWebContentProcessDidTerminate(_: WKWebView) {
        MainActor.assumeIsolated {
            self.isBroken = true
            self.onLoadEnd?(.failure(SnapshotError.webContentTerminated))
            self.onLoadEnd = nil
        }
    }

    private func endLoad(_ navigation: WKNavigation?, _ result: Result<Bool, Error>) {
        // A late callback for a page this view has since replaced is not news.
        guard navigation === self.navigation else {
            return
        }
        self.onLoadEnd?(result)
        self.onLoadEnd = nil
    }
}

/// Run `start`, which must eventually call the closure it is handed, and return
/// what it reports - or `nil` if `deadline` passes first.
///
/// WebKit's completion handlers cannot be cancelled, so a structured
/// `withTaskGroup` race would wait for the loser anyway. Here the first of
/// "WebKit answered" and "the deadline passed" resumes the caller, and the other
/// is ignored when it arrives.
@MainActor
func race<T>(
    until deadline: ContinuousClock.Instant,
    _ start: (@escaping (Result<T, Error>) -> Void) -> Void
) async throws -> T? {
    let once = ResumeOnce<T?>()
    return try await withCheckedThrowingContinuation { continuation in
        once.continuation = continuation
        let timer = Task { @MainActor in
            try? await Task.sleep(until: deadline, clock: .continuous)
            once.resume(with: .success(nil))
        }
        start { result in
            MainActor.assumeIsolated {
                timer.cancel()
                once.resume(with: result.map { $0 })
            }
        }
    }
}

/// A continuation that only the first caller resumes; later calls are dropped.
@MainActor
private final class ResumeOnce<T> {
    var continuation: CheckedContinuation<T, Error>?

    func resume(with result: Result<T, Error>) {
        self.continuation?.resume(with: result)
        self.continuation = nil
    }
}

/// A small pool of offscreen web views that turns preview documents into images.
///
/// # Why a pool
///
/// Each `WKWebView` costs a WebContent process launch on first use - about
/// 150-200 ms, more than everything else a thumbnail does put together - and
/// Finder asks for thumbnails in bursts (a folder in icon view is dozens at
/// once). Two views, reused for the life of the extension process, pay that
/// launch twice and then render back to back. Requests past two wait their turn
/// in arrival order; each one's deadline starts when it gets a view, not when it
/// joined the queue, so a burst is not failed just for being a burst.
///
/// # No network
///
/// Every view carries a content rule list blocking everything but `cid:`,
/// `data:`, `about:` and `blob:`. It is the only network barrier: the
/// extension has to carry `network.client`, because WebKit's GPU process
/// exits at launch in a sandboxed host without it (MBRThumbnail.entitlements
/// has the log line). A note's remote image or iframe then renders empty
/// immediately instead of holding the `load` event - and the snapshot - for as
/// long as the remote server takes. The rules are compiled once per process;
/// if they cannot be, nothing is rendered at all.
@MainActor
final class WebSnapshotter {
    static let shared = WebSnapshotter()

    static let poolSize = 2

    /// Load and settle budget for one page, from the moment it gets a view.
    static let renderDeadline: Duration = .milliseconds(2500)

    /// Extra time `takeSnapshot` gets after the render deadline.
    static let snapshotGrace: Duration = .milliseconds(750)

    /// Schemes a rendered page legitimately uses with no network: attachments,
    /// inline images and fonts, the document itself, and script-made blobs.
    static let localSchemes = ["cid", "data", "about", "blob"]

    /// Content blocker: everything is blocked, then the local schemes are let
    /// back through - one rule each, because WebKit's rule regexes have no
    /// alternation (`(a|b)` fails to compile).
    static var contentRulesJSON: String {
        let allow = localSchemes.map {
            #"{"trigger": {"url-filter": "^\#($0):"}, "action": {"type": "ignore-previous-rules"}}"#
        }
        let block = #"{"trigger": {"url-filter": ".*"}, "action": {"type": "block"}}"#
        return "[" + ([block] + allow).joined(separator: ",") + "]"
    }

    private var contentRules: WKContentRuleList?
    private var idle: [SnapshotView] = []
    private var created = 0
    private var waiters: [CheckedContinuation<SnapshotView, Never>] = []

    func snapshot(_ page: PreviewDocument, pixelWidth: CGFloat) async throws -> NSImage {
        let rules = try await loadContentRules()
        let view = await acquire(rules)
        defer { release(view, rules) }
        return try await view.snapshot(
            page,
            pixelWidth: pixelWidth,
            deadline: .now + Self.renderDeadline,
            snapshotGrace: Self.snapshotGrace
        )
    }

    private func loadContentRules() async throws -> WKContentRuleList {
        if let contentRules {
            return contentRules
        }
        guard let store = WKContentRuleListStore.default() else {
            throw SnapshotError.contentRulesUnavailable(nil)
        }
        do {
            guard let rules = try await store.compileContentRuleList(
                forIdentifier: "mbr-thumbnail-no-network",
                encodedContentRuleList: Self.contentRulesJSON
            ) else {
                throw SnapshotError.contentRulesUnavailable(nil)
            }
            contentRules = rules
            return rules
        } catch let error as SnapshotError {
            throw error
        } catch {
            throw SnapshotError.contentRulesUnavailable(error)
        }
    }

    private func acquire(_ rules: WKContentRuleList) async -> SnapshotView {
        if let view = idle.popLast() {
            return view
        }
        if self.created < Self.poolSize {
            self.created += 1
            return SnapshotView(contentRules: rules)
        }
        return await withCheckedContinuation { self.waiters.append($0) }
    }

    private func release(_ view: SnapshotView, _ rules: WKContentRuleList) {
        // A broken view is dropped, which tears down its WebContent process,
        // and its slot is refilled with a fresh one only if someone is waiting.
        var next: SnapshotView? = view
        if view.isBroken {
            os_log(.error, log: thumbnailLog, "discarding a web view whose page hung or crashed")
            view.webView.stopLoading()
            self.created -= 1
            next = nil
        }
        guard !self.waiters.isEmpty else {
            if let next {
                self.idle.append(next)
            }
            return
        }
        let replacement = next ?? {
            self.created += 1
            return SnapshotView(contentRules: rules)
        }()
        self.waiters.removeFirst().resume(returning: replacement)
    }
}
