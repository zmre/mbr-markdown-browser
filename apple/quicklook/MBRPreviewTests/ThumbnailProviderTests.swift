// ThumbnailProviderTests.swift
// MBR Markdown QuickLook Extension Tests
//
// Unit tests for the thumbnail extension.
//
// As with the preview, `provideThumbnail(for:_:)` is not directly callable:
// QLFileThumbnailRequest has no public initializer. It is a thin switch over
// `renderThumbnail` (whose rules the Rust tests in src/quicklook.rs own) and
// `WebSnapshotter`, so those are tested here, end to end where WebKit allows.

import AppKit
import Network
import WebKit
import XCTest

@MainActor
final class ThumbnailProviderTests: XCTestCase {
    /// A temp directory that is removed when the test finishes.
    private func makeTempDirectory(_ label: String) throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-thumb-test-\(label)-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    /// Solid red in the bitmap's own color space. A catalog color such as
    /// `NSColor.red` cannot be converted by `setColor(_:atX:y:)` and silently
    /// leaves the pixel blank.
    private static let red = NSColor(deviceRed: 1, green: 0, blue: 0, alpha: 1)

    /// Write a solid-color PNG of `size` pixels to `url`.
    private func writePNG(_ url: URL, color: NSColor = red, size: Int = 64) throws {
        let rep = try XCTUnwrap(NSBitmapImageRep(
            bitmapDataPlanes: nil,
            pixelsWide: size,
            pixelsHigh: size,
            bitsPerSample: 8,
            samplesPerPixel: 4,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: .deviceRGB,
            bytesPerRow: 0,
            bitsPerPixel: 0
        ))
        for x in 0 ..< size {
            for y in 0 ..< size {
                rep.setColor(color, atX: x, y: y)
            }
        }
        try XCTUnwrap(rep.representation(using: .png, properties: [:])).write(to: url)
    }

    /// The color of the pixel at the center of `image`.
    private func centerColor(_ image: NSImage) throws -> NSColor {
        let tiff = try XCTUnwrap(image.tiffRepresentation)
        let rep = try XCTUnwrap(NSBitmapImageRep(data: tiff))
        let color = try XCTUnwrap(rep.colorAt(x: rep.pixelsWide / 2, y: rep.pixelsHigh / 2))
        return try XCTUnwrap(color.usingColorSpace(.deviceRGB))
    }

    // MARK: - Geometry

    func testContextSize_fitsPageAspectInsideSquare() {
        let size = ThumbnailGeometry.contextSize(fitting: CGSize(width: 512, height: 512))
        XCTAssertEqual(size.height, 512)
        XCTAssertEqual(size.width, 395, "800 x 1035.3 scaled to 512 tall, rounded down")
    }

    func testContextSize_fitsPageAspectInsideWideRect() {
        let size = ThumbnailGeometry.contextSize(fitting: CGSize(width: 1000, height: 200))
        XCTAssertEqual(size.height, 200)
        XCTAssertLessThan(size.width, 200, "a page is taller than it is wide")
    }

    func testContextSize_degenerateRequestIsOnePoint() {
        XCTAssertEqual(ThumbnailGeometry.contextSize(fitting: .zero), CGSize(width: 1, height: 1))
        XCTAssertEqual(
            ThumbnailGeometry.contextSize(fitting: CGSize(width: 0.1, height: 0.1)),
            CGSize(width: 1, height: 1)
        )
    }

    // MARK: - cid: scheme handler

    func testCidIsNotABuiltInScheme() {
        // WebKit refuses a handler for a scheme it handles itself. If this ever
        // flips, `setURLSchemeHandler` throws and every page thumbnail fails.
        XCTAssertFalse(WKWebView.handlesURLScheme(AttachmentSchemeHandler.scheme))
    }

    func testAttachmentID() {
        XCTAssertEqual(AttachmentSchemeHandler.attachmentID(for: URL(string: "cid:mbr-asset-0")), "mbr-asset-0")
        XCTAssertEqual(AttachmentSchemeHandler.attachmentID(for: URL(string: "CID:mbr-asset-1")), "mbr-asset-1")
        XCTAssertNil(AttachmentSchemeHandler.attachmentID(for: URL(string: "cid:")))
        XCTAssertNil(AttachmentSchemeHandler.attachmentID(for: URL(string: "https://example.com/cid:x")))
        XCTAssertNil(AttachmentSchemeHandler.attachmentID(for: URL(string: "file:///etc/passwd")))
        XCTAssertNil(AttachmentSchemeHandler.attachmentID(for: nil))
    }

    func testImageMIMEType_servesImagesOnly() {
        XCTAssertEqual(AttachmentSchemeHandler.imageMIMEType(forPath: "/r/a.png"), "image/png")
        XCTAssertEqual(AttachmentSchemeHandler.imageMIMEType(forPath: "/r/a.JPG"), "image/jpeg")
        XCTAssertNotNil(AttachmentSchemeHandler.imageMIMEType(forPath: "/r/a.svg"))
        XCTAssertNil(AttachmentSchemeHandler.imageMIMEType(forPath: "/r/movie.mp4"))
        XCTAssertNil(AttachmentSchemeHandler.imageMIMEType(forPath: "/r/doc.pdf"))
        XCTAssertNil(AttachmentSchemeHandler.imageMIMEType(forPath: "/r/no-extension"))
    }

    // MARK: - Content rules

    func testContentRulesCompile() async throws {
        let json = WebSnapshotter.contentRulesJSON
        let parsed = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[String: Any]]
        XCTAssertEqual(parsed?.count, 1 + WebSnapshotter.localSchemes.count)

        let store = try XCTUnwrap(WKContentRuleListStore.default())
        let rules = try await store.compileContentRuleList(
            forIdentifier: "mbr-thumbnail-test-rules",
            encodedContentRuleList: json
        )
        XCTAssertNotNil(rules, "WebKit's rule grammar is narrower than JS regex; it must accept these")
    }

    // MARK: - renderThumbnail (FFI)

    func testRenderThumbnail_coverImage() throws {
        let repo = try makeTempDirectory("cover")
        try FileManager.default.createDirectory(
            at: repo.appendingPathComponent(".mbr"),
            withIntermediateDirectories: true
        )
        try self.writePNG(repo.appendingPathComponent("cover.png"))
        let note = repo.appendingPathComponent("note.md")
        try "---\ncover: cover.png\n---\n# Note\n".write(to: note, atomically: true, encoding: .utf8)

        guard case let .image(path) = try renderThumbnail(filePath: note.path, configRoot: nil) else {
            return XCTFail("expected the cover image")
        }
        XCTAssertEqual(
            URL(fileURLWithPath: path).resolvingSymlinksInPath(),
            repo.appendingPathComponent("cover.png").resolvingSymlinksInPath()
        )
    }

    // MARK: - Badge

    /// Both kinds of reply carry the "MD" file-type badge: a note with a cover
    /// image is still a markdown file.
    func testReplies_carryTheMarkdownBadge() throws {
        let dir = try makeTempDirectory("badge")
        let png = dir.appendingPathComponent("cover.png")
        try self.writePNG(png)
        XCTAssertEqual(ThumbnailProvider.reply(coverImageAt: png.path).extensionBadge, "MD")

        let image = NSImage(size: NSSize(width: 10, height: 10))
        let page = ThumbnailProvider.reply(drawing: image, contextSize: CGSize(width: 10, height: 13))
        XCTAssertEqual(page.extensionBadge, "MD")
    }

    func testRenderThumbnail_pageWithoutCover() throws {
        let repo = try makeTempDirectory("page")
        let note = repo.appendingPathComponent("note.md")
        try "# Just text\n".write(to: note, atomically: true, encoding: .utf8)

        guard case let .page(page) = try renderThumbnail(filePath: note.path, configRoot: nil) else {
            return XCTFail("expected a page")
        }
        XCTAssertTrue(page.html.contains("Just text"))
    }

    func testRenderThumbnail_rejectsNonMarkdown() throws {
        let repo = try makeTempDirectory("text")
        let file = repo.appendingPathComponent("notes.txt")
        try "plain".write(to: file, atomically: true, encoding: .utf8)

        XCTAssertThrowsError(try renderThumbnail(filePath: file.path, configRoot: nil))
    }

    // MARK: - WebSnapshotter

    func testSnapshot_servesCidAttachmentAtRequestedWidth() async throws {
        let dir = try makeTempDirectory("snapshot")
        let png = dir.appendingPathComponent("red.png")
        try self.writePNG(png)
        let page = PreviewDocument(
            html: """
            <html><body style="margin:0">
            <img src="cid:red" loading="lazy" style="display:block;width:100vw;height:100vh">
            </body></html>
            """,
            attachments: [PreviewAttachment(id: "red", path: png.path)]
        )

        let image = try await WebSnapshotter.shared.snapshot(page, pixelWidth: 200)
        XCTAssertEqual(SnapshotView.pixelWidth(of: image), 200)

        // A lazy image too: offscreen, lazy loading never fires on its own.
        let center = try centerColor(image)
        XCTAssertGreaterThan(center.redComponent, 0.8)
        XCTAssertLessThan(center.greenComponent, 0.2)
    }

    func testSnapshot_isFullResolutionAt2xAnd3x() async throws {
        let page = PreviewDocument(html: "<html><body><h1>Title</h1></body></html>", attachments: [])
        // A 512 pt request fits a 395 pt wide page; at 2x and 3x that is the
        // pixel width QuickLook's context has, so the snapshot must match it.
        let points = ThumbnailGeometry.contextSize(fitting: CGSize(width: 512, height: 512)).width
        for scale in [2.0, 3.0] as [CGFloat] {
            let image = try await WebSnapshotter.shared.snapshot(page, pixelWidth: points * scale)
            XCTAssertEqual(
                SnapshotView.pixelWidth(of: image),
                points * scale,
                accuracy: 1,
                "snapshot at \(scale)x"
            )
        }
    }

    /// The content rule list is the thumbnail's only network barrier: the
    /// extension carries `network.client` because WebKit's GPU process will
    /// not start in a sandbox without it (MBRThumbnail.entitlements). So no
    /// remote load may leave the page, whatever kind of reference makes it.
    func testSnapshot_makesNoRemoteRequests() async throws {
        let listener = try ConnectionCounter()
        defer { listener.cancel() }
        let port = try await listener.ready()
        let origin = "http://127.0.0.1:\(port)"
        let page = PreviewDocument(
            html: """
            <html><head><link rel="stylesheet" href="\(origin)/style.css"></head><body>
            <img src="\(origin)/eager.png">
            <img src="\(origin)/lazy.png" loading="lazy">
            <iframe src="\(origin)/frame.html"></iframe>
            <script>fetch("\(origin)/fetch.json").catch(() => {});</script>
            </body></html>
            """,
            attachments: []
        )

        _ = try await WebSnapshotter.shared.snapshot(page, pixelWidth: 100)
        // Give any request that did escape time to connect.
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(listener.count, 0, "a thumbnail page reached the network")
    }

    /// Control for the test above: the counter does see a real request, so
    /// its zero means something.
    func testConnectionCounter_countsARealRequest() async throws {
        let listener = try ConnectionCounter()
        defer { listener.cancel() }
        let port = try await listener.ready()
        let url = try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/probe"))
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 1
        _ = try? await URLSession(configuration: configuration).data(from: url)
        XCTAssertGreaterThan(listener.count, 0)
    }

    func testSnapshot_unlistedCidIsNotServed() async throws {
        let dir = try makeTempDirectory("unlisted")
        let png = dir.appendingPathComponent("red.png")
        try self.writePNG(png)
        // The file exists and the page names it, but not through the list.
        let page = PreviewDocument(
            html: """
            <html><body style="margin:0;background:#fff">
            <img src="cid:other" style="display:block;width:100vw;height:100vh">
            </body></html>
            """,
            attachments: [PreviewAttachment(id: "red", path: png.path)]
        )

        let image = try await WebSnapshotter.shared.snapshot(page, pixelWidth: 100)
        // Not red. What is there instead is WebKit's grey broken-image box.
        let center = try centerColor(image)
        XCTAssertLessThan(center.redComponent - center.greenComponent, 0.3, "the red file must not be served")
    }

    func testSnapshot_isLightEvenInDarkMode() async throws {
        let page = PreviewDocument(
            html: """
            <html><head><meta name="color-scheme" content="light dark"><style>
            body { margin: 0; height: 100vh; background: #00ff00; }
            @media (prefers-color-scheme: dark) { body { background: #0000ff; } }
            </style></head><body></body></html>
            """,
            attachments: []
        )
        let previous = NSApp?.appearance
        NSApp?.appearance = NSAppearance(named: .darkAqua)
        defer { NSApp?.appearance = previous }

        let image = try await WebSnapshotter.shared.snapshot(page, pixelWidth: 100)
        let center = try centerColor(image)
        XCTAssertGreaterThan(center.greenComponent, 0.8)
        XCTAssertLessThan(center.blueComponent, 0.2)
    }

    func testSnapshot_hungPageTimesOutAndPoolRecovers() async throws {
        let hung = PreviewDocument(
            html: "<html><body><p>before</p><script>for (;;) {}</script></body></html>",
            attachments: []
        )
        let start = ContinuousClock.now
        do {
            _ = try await WebSnapshotter.shared.snapshot(hung, pixelWidth: 100)
            XCTFail("a page that never yields cannot be snapshotted")
        } catch SnapshotError.timedOut {
            // expected
        }
        let elapsed = ContinuousClock.now - start
        XCTAssertLessThan(
            elapsed,
            WebSnapshotter.renderDeadline + WebSnapshotter.snapshotGrace + .seconds(1),
            "the deadline bounds even a page that never yields"
        )

        // The hung view was discarded; the next page gets a working one.
        let fine = PreviewDocument(html: "<html><body>ok</body></html>", attachments: [])
        let image = try await WebSnapshotter.shared.snapshot(fine, pixelWidth: 100)
        XCTAssertEqual(image.representations.first?.pixelsWide, 100)
    }
}

/// A loopback TCP listener that counts the connections it accepts and answers
/// none of them: enough to tell whether a page tried to reach the network.
final class ConnectionCounter: @unchecked Sendable {
    private let listener: NWListener
    private let queue = DispatchQueue(label: "mbr.test.connection-counter")
    private var accepted = 0

    init() throws {
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        self.listener = try NWListener(using: parameters)
        self.listener.newConnectionHandler = { [weak self] connection in
            guard let self else {
                return
            }
            self.accepted += 1
            connection.cancel()
        }
    }

    /// Connections accepted so far.
    var count: Int {
        self.queue.sync { self.accepted }
    }

    /// Start listening and return the port.
    func ready() async throws -> UInt16 {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<UInt16, Error>) in
            var resumed = false
            self.listener.stateUpdateHandler = { [listener] state in
                guard !resumed else {
                    return
                }
                switch state {
                case .ready:
                    resumed = true
                    continuation.resume(returning: listener.port?.rawValue ?? 0)
                case let .failed(error):
                    resumed = true
                    continuation.resume(throwing: error)
                default:
                    break
                }
            }
            self.listener.start(queue: self.queue)
        }
    }

    func cancel() {
        self.listener.cancel()
    }
}
