// PreviewViewControllerTests.swift
// MBR Markdown QuickLook Extension Tests
//
// Unit tests for PreviewViewController
//
// Note: Tests that verify WebView content via JavaScript are skipped because
// WKWebView's WebContent process cannot run in the sandboxed test environment.
// The core rendering logic is tested via Rust unit tests in src/quicklook.rs.

import WebKit
import XCTest

class PreviewViewControllerTests: XCTestCase {
    var viewController: PreviewViewController!

    override func setUp() {
        super.setUp()
        self.viewController = PreviewViewController()
    }

    override func tearDown() {
        self.viewController = nil
        super.tearDown()
    }

    // MARK: - View Creation Tests

    func testLoadView_createsWebView() {
        // When
        self.viewController.loadView()

        // Then
        XCTAssertNotNil(self.viewController.view, "View should be created")
        XCTAssertTrue(self.viewController.view is WKWebView, "View should be a WKWebView")

        guard let webView = self.viewController.view as? WKWebView else {
            XCTFail("View should be a WKWebView")
            return
        }
        XCTAssertTrue(webView.autoresizingMask.contains(.width), "WebView should autoresize width")
        XCTAssertTrue(webView.autoresizingMask.contains(.height), "WebView should autoresize height")
        XCTAssertNotNil(webView.navigationDelegate, "Navigation delegate should be set")
    }

    // MARK: - Config Root Finding Tests

    func testFindConfigRoot_withMbrDirectory() throws {
        // Given
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        // Create .mbr directory
        let mbrDir = tempDir.appendingPathComponent(".mbr")
        try FileManager.default.createDirectory(at: mbrDir, withIntermediateDirectories: true)

        // Create a subdirectory with a test file
        let subDir = tempDir.appendingPathComponent("docs")
        try FileManager.default.createDirectory(at: subDir, withIntermediateDirectories: true)
        let testFile = subDir.appendingPathComponent("test.md")
        try "# Test".write(to: testFile, atomically: true, encoding: .utf8)

        // When - use FFI function (free function, not a method on the controller)
        let result = findConfigRoot(filePath: testFile.path)

        // Then
        XCTAssertEqual(result, tempDir.path, "Should return the directory containing .mbr")
    }

    func testFindConfigRoot_withoutMbrDirectory() throws {
        // Given
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-no-config-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let testFile = tempDir.appendingPathComponent("test.md")
        try "# Test".write(to: testFile, atomically: true, encoding: .utf8)

        // When - use FFI function; falls back to file's parent dir when no markers found
        let result = findConfigRoot(filePath: testFile.path)

        // Then - should fall back to the file's parent directory
        XCTAssertEqual(result, tempDir.path, "Should fall back to file's parent directory when no markers found")
    }

    func testFindConfigRoot_nestedDirectories() throws {
        // Given - create deep directory structure
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-nested-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        // Create .mbr at root
        let mbrDir = tempDir.appendingPathComponent(".mbr")
        try FileManager.default.createDirectory(at: mbrDir, withIntermediateDirectories: true)

        // Create deeply nested file
        let deepPath = tempDir
            .appendingPathComponent("level1")
            .appendingPathComponent("level2")
            .appendingPathComponent("level3")
        try FileManager.default.createDirectory(at: deepPath, withIntermediateDirectories: true)
        let testFile = deepPath.appendingPathComponent("deep.md")
        try "# Deep test".write(to: testFile, atomically: true, encoding: .utf8)

        // When - use FFI function
        let result = findConfigRoot(filePath: testFile.path)

        // Then
        XCTAssertEqual(result, tempDir.path, "Should return the root directory containing .mbr")
    }

    // MARK: - Preview Rendering Tests

    // These tests verify that rendering completes without error.
    // Content verification is done via Rust unit tests in src/quicklook.rs.

    func testRenderPreview_completesWithoutError() throws {
        // Given
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-render-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let testFile = tempDir.appendingPathComponent("test.md")
        let markdownContent = """
        # Test Heading

        This is a test paragraph.
        """
        try markdownContent.write(to: testFile, atomically: true, encoding: .utf8)

        // When
        self.viewController.loadView()
        let expectation = self.expectation(description: "Preview should render")

        self.viewController.preparePreviewOfFile(at: testFile) { error in
            // Then
            XCTAssertNil(error, "Should render without error")
            expectation.fulfill()
        }

        waitForExpectations(timeout: 5.0)
    }

    func testRenderPreview_withCustomCSS_completesWithoutError() throws {
        // Given
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-custom-css-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        // Create .mbr directory with custom theme.css
        let mbrDir = tempDir.appendingPathComponent(".mbr")
        try FileManager.default.createDirectory(at: mbrDir, withIntermediateDirectories: true)

        let themeCss = mbrDir.appendingPathComponent("theme.css")
        let customCSS = """
        /* Custom MBR Theme */
        body {
            background-color: #custom-color;
            font-family: "Custom Font", serif;
        }
        """
        try customCSS.write(to: themeCss, atomically: true, encoding: .utf8)

        let testFile = tempDir.appendingPathComponent("test.md")
        try "# Custom Theme Test".write(to: testFile, atomically: true, encoding: .utf8)

        // When
        self.viewController.loadView()
        let expectation = self.expectation(description: "Preview with custom CSS should render")

        self.viewController.preparePreviewOfFile(at: testFile) { error in
            // Then
            XCTAssertNil(error, "Should render without error")
            expectation.fulfill()
        }

        waitForExpectations(timeout: 5.0)
    }

    func testRenderPreview_withFrontmatter_completesWithoutError() throws {
        // Given
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-frontmatter-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let testFile = tempDir.appendingPathComponent("frontmatter.md")
        let markdownContent = """
        ---
        title: Test Page
        author: John Doe
        tags: [test, example]
        ---

        # Content

        This is the actual content.
        """
        try markdownContent.write(to: testFile, atomically: true, encoding: .utf8)

        // When
        self.viewController.loadView()
        let expectation = self.expectation(description: "Frontmatter should be parsed")

        self.viewController.preparePreviewOfFile(at: testFile) { error in
            // Then
            XCTAssertNil(error, "Should render without error")
            expectation.fulfill()
        }

        waitForExpectations(timeout: 5.0)
    }

    // MARK: - Error Handling Tests

    func testLoadErrorHTML_doesNotCrash() {
        // Given
        self.viewController.loadView()
        let dangerousMessage = "<script>alert('XSS')</script> & \"quotes\" > < test"

        // When/Then - should not crash
        self.viewController.loadErrorHTML(message: dangerousMessage)

        // Verify WebView received the HTML (doesn't crash)
        XCTAssertNotNil(self.viewController.view, "View should still exist after loading error HTML")
    }

    func testLoadErrorHTML_withSimpleMessage_doesNotCrash() {
        // Given
        self.viewController.loadView()
        let errorMessage = "Test error message"

        // When/Then - should not crash
        self.viewController.loadErrorHTML(message: errorMessage)

        // Verify WebView received the HTML (doesn't crash)
        XCTAssertNotNil(self.viewController.view, "View should still exist after loading error HTML")
    }

    // MARK: - Scheme Handler Containment Tests

    /// Creates a workspace laid out as:
    ///
    /// ```text
    /// <tmp>/          <- "outside" the repo; escape targets live here
    ///   repo/         <- the previewed repository root
    ///     note.md
    ///   secret.txt
    /// ```
    ///
    /// Returns (tempDir, resolved repo root). Caller must remove tempDir.
    private func makeContainmentFixture() throws -> (URL, String) {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-contain-\(UUID().uuidString)")
        let repo = tempDir.appendingPathComponent("repo")
        try FileManager.default.createDirectory(at: repo, withIntermediateDirectories: true)
        try "# Note".write(to: repo.appendingPathComponent("note.md"), atomically: true, encoding: .utf8)
        try "secret".write(to: tempDir.appendingPathComponent("secret.txt"), atomically: true, encoding: .utf8)

        let root = try XCTUnwrap(MBRFileSchemeHandler.realPath(repo.path))
        return (tempDir, root)
    }

    func testIsContained_acceptsFileInsideRoot() throws {
        let (tempDir, root) = try makeContainmentFixture()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let inside = try XCTUnwrap(MBRFileSchemeHandler.realPath(root + "/note.md"))
        XCTAssertTrue(MBRFileSchemeHandler.isContained(inside, in: root))
    }

    func testIsContained_rejectsDotDotTraversal() throws {
        let (tempDir, root) = try makeContainmentFixture()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        // realPath collapses the `..`, which is exactly why containment must be
        // checked on the resolved path and never on the requested string.
        let escaped = try XCTUnwrap(MBRFileSchemeHandler.realPath(root + "/../secret.txt"))
        XCTAssertFalse(MBRFileSchemeHandler.isContained(escaped, in: root))
    }

    func testIsContained_rejectsUnrelatedAbsolutePath() throws {
        let (tempDir, root) = try makeContainmentFixture()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let passwd = try XCTUnwrap(MBRFileSchemeHandler.realPath("/etc/passwd"))
        XCTAssertFalse(MBRFileSchemeHandler.isContained(passwd, in: root))
    }

    func testIsContained_rejectsSymlinkEscapingRoot() throws {
        let (tempDir, root) = try makeContainmentFixture()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        // A symlink inside the repo pointing at a file outside it.
        try FileManager.default.createSymbolicLink(
            atPath: root + "/leak.png",
            withDestinationPath: tempDir.appendingPathComponent("secret.txt").path
        )

        let resolved = try XCTUnwrap(MBRFileSchemeHandler.realPath(root + "/leak.png"))
        XCTAssertFalse(MBRFileSchemeHandler.isContained(resolved, in: root))
    }

    func testIsContained_rejectsSiblingRootWithSharedPrefix() {
        // Plain string prefixing would wrongly accept this.
        XCTAssertFalse(MBRFileSchemeHandler.isContained("/notes-evil/secret", in: "/notes"))
        XCTAssertTrue(MBRFileSchemeHandler.isContained("/notes/ok.png", in: "/notes"))
    }

    func testIsContained_rejectsRootItself() {
        // The root is a directory, never a servable asset.
        XCTAssertFalse(MBRFileSchemeHandler.isContained("/notes", in: "/notes"))
    }

    func testRealPath_returnsNilForMissingPath() {
        XCTAssertNil(MBRFileSchemeHandler.realPath("/nonexistent-\(UUID().uuidString)/file.png"))
    }

    func testSchemeHandler_startsWithNoAllowedRoot() {
        // Fail closed: until preparePreviewOfFile establishes a root, the
        // handler must not be willing to serve anything.
        XCTAssertNil(MBRFileSchemeHandler().allowedRoot)
    }

    func testPreparePreview_confinesSchemeHandlerToRepositoryRoot() throws {
        // Given
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("mbr-test-confine-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let mbrDir = tempDir.appendingPathComponent(".mbr")
        try FileManager.default.createDirectory(at: mbrDir, withIntermediateDirectories: true)

        let testFile = tempDir.appendingPathComponent("test.md")
        try "# Test".write(to: testFile, atomically: true, encoding: .utf8)

        // When
        self.viewController.loadView()
        let expectation = self.expectation(description: "Preview should render")
        self.viewController.preparePreviewOfFile(at: testFile) { _ in expectation.fulfill() }
        waitForExpectations(timeout: 5.0)

        // Then - the handler is pinned to the repo root, and nothing above it
        // can be read.
        let root = try XCTUnwrap(self.viewController.fileSchemeHandler.allowedRoot)
        XCTAssertEqual(root, MBRFileSchemeHandler.realPath(tempDir.path))
        XCTAssertTrue(MBRFileSchemeHandler.isContained(root + "/test.md", in: root))

        let outside = try XCTUnwrap(MBRFileSchemeHandler.realPath("/etc/passwd"))
        XCTAssertFalse(MBRFileSchemeHandler.isContained(outside, in: root))
    }

    func testRenderPreview_withNonexistentFile_completesWithoutCrashing() {
        // Given
        let nonexistentFile = URL(fileURLWithPath: "/nonexistent/path/to/file.md")

        // When
        self.viewController.loadView()
        let expectation = self.expectation(description: "Should handle nonexistent file")

        self.viewController.preparePreviewOfFile(at: nonexistentFile) { _ in
            // Then - completion handler should still be called
            expectation.fulfill()
        }

        waitForExpectations(timeout: 5.0)

        // Verify view controller didn't crash
        XCTAssertNotNil(self.viewController.view, "View should still exist after handling error")
    }
}
