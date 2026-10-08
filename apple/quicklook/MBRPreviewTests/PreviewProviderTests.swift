// PreviewProviderTests.swift
// MBR Markdown QuickLook Extension Tests
//
// Unit tests for PreviewProvider.
//
// `providePreview(for:)` itself is not directly testable: QLFilePreviewRequest
// has no public initializer, so there is no way to hand the provider a request
// from a test. What it does is therefore tested through its two halves - the
// FFI render call and `loadAttachments` - plus the Rust unit tests in
// src/quicklook.rs, which own the HTML and the containment rules.

import QuickLookUI
import UniformTypeIdentifiers
import XCTest

class PreviewProviderTests: XCTestCase {
    /// A temp directory that is removed when the test finishes.
    private func makeTempDirectory(_ label: String) throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-\(label)-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    // MARK: - Config Root Finding Tests

    func testFindConfigRoot_withMbrDirectory() throws {
        let tempDir = try makeTempDirectory("config-root")
        try FileManager.default.createDirectory(
            at: tempDir.appendingPathComponent(".mbr"),
            withIntermediateDirectories: true
        )

        let subDir = tempDir.appendingPathComponent("docs")
        try FileManager.default.createDirectory(at: subDir, withIntermediateDirectories: true)
        let testFile = subDir.appendingPathComponent("test.md")
        try "# Test".write(to: testFile, atomically: true, encoding: .utf8)

        XCTAssertEqual(
            findConfigRoot(filePath: testFile.path),
            tempDir.path,
            "Should return the directory containing .mbr"
        )
    }

    func testFindConfigRoot_withoutMbrDirectory() throws {
        let tempDir = try makeTempDirectory("no-config")
        let testFile = tempDir.appendingPathComponent("test.md")
        try "# Test".write(to: testFile, atomically: true, encoding: .utf8)

        XCTAssertEqual(
            findConfigRoot(filePath: testFile.path),
            tempDir.path,
            "Should fall back to file's parent directory when no markers found"
        )
    }

    func testFindConfigRoot_nestedDirectories() throws {
        let tempDir = try makeTempDirectory("nested")
        try FileManager.default.createDirectory(
            at: tempDir.appendingPathComponent(".mbr"),
            withIntermediateDirectories: true
        )

        let deepPath = tempDir
            .appendingPathComponent("level1")
            .appendingPathComponent("level2")
            .appendingPathComponent("level3")
        try FileManager.default.createDirectory(at: deepPath, withIntermediateDirectories: true)
        let testFile = deepPath.appendingPathComponent("deep.md")
        try "# Deep test".write(to: testFile, atomically: true, encoding: .utf8)

        XCTAssertEqual(
            findConfigRoot(filePath: testFile.path),
            tempDir.path,
            "Should return the root directory containing .mbr"
        )
    }

    // MARK: - Rendering Tests

    func testRenderPreview_producesSelfContainedHTML() throws {
        let tempDir = try makeTempDirectory("render")
        let testFile = tempDir.appendingPathComponent("test.md")
        try "# Test Heading\n\nA paragraph.".write(to: testFile, atomically: true, encoding: .utf8)

        let document = try renderPreview(filePath: testFile.path, configRoot: tempDir.path)

        XCTAssertTrue(document.html.contains("Test Heading"))
        XCTAssertTrue(document.html.contains("<style>"), "CSS must be inlined")
        XCTAssertTrue(document.attachments.isEmpty, "A note with no local assets attaches nothing")
    }

    func testRenderPreview_withNonexistentFile_throws() {
        XCTAssertThrowsError(
            try renderPreview(filePath: "/nonexistent/path/to/file.md", configRoot: "/tmp")
        )
    }

    // MARK: - Attachment Tests

    /// The end-to-end contract this extension depends on: every `cid:` URL in
    /// the HTML must name an attachment the provider actually loaded. If these
    /// two drift apart the preview shows a broken image and nothing reports an
    /// error.
    func testAttachments_everyCidInTheHTMLIsLoadable() throws {
        let tempDir = try makeTempDirectory("attachments")
        let imagesDir = tempDir.appendingPathComponent("images")
        try FileManager.default.createDirectory(at: imagesDir, withIntermediateDirectories: true)
        // A 1x1 transparent GIF, so the bytes are a real image of a real type.
        let gif = Data(base64Encoded: "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7")
        try XCTUnwrap(gif).write(to: imagesDir.appendingPathComponent("pixel.gif"))

        let testFile = tempDir.appendingPathComponent("note.md")
        try "![pixel](/images/pixel.gif)".write(to: testFile, atomically: true, encoding: .utf8)

        let document = try renderPreview(filePath: testFile.path, configRoot: tempDir.path)
        let attachment = try XCTUnwrap(document.attachments.first, "the image must be attached")

        XCTAssertTrue(
            document.html.contains("cid:\(attachment.id)"),
            "the HTML must address the attachment by its id"
        )

        let loaded = PreviewProvider.loadAttachments(document.attachments)
        let entry = try XCTUnwrap(loaded[attachment.id])
        XCTAssertEqual(entry.data, gif, "the attachment must carry the file's bytes")
        XCTAssertEqual(entry.contentType, .gif, "content type comes from the extension")
    }

    func testLoadAttachments_skipsUnreadableFiles() throws {
        let tempDir = try makeTempDirectory("unreadable")
        let missing = tempDir.appendingPathComponent("gone.png").path

        let loaded = PreviewProvider.loadAttachments([
            PreviewAttachment(id: "mbr-asset-0", path: missing)
        ])

        XCTAssertTrue(loaded.isEmpty, "a file that cannot be read is dropped, not faked")
    }

    func testLoadAttachments_unknownExtensionFallsBackToData() throws {
        let tempDir = try makeTempDirectory("unknown-ext")
        let file = tempDir.appendingPathComponent("thing.zzzznotatype")
        try Data("bytes".utf8).write(to: file)

        let loaded = PreviewProvider.loadAttachments([
            PreviewAttachment(id: "mbr-asset-0", path: file.path)
        ])

        XCTAssertEqual(try XCTUnwrap(loaded["mbr-asset-0"]).contentType, .data)
    }

    // MARK: - Error Handling Tests

    func testErrorHTML_escapesMarkup() {
        let html = PreviewProvider.errorHTML(message: "<script>alert('XSS')</script> & \"q\" > <")

        XCTAssertFalse(html.contains("<script>alert"), "message markup must not reach the document")
        XCTAssertTrue(html.contains("&lt;script&gt;"))
        XCTAssertTrue(html.contains("&amp;"))
        XCTAssertTrue(html.contains("Preview Error"))
    }
}
