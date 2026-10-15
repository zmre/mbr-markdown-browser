import XCTest

/// Drives what only a tap can open: the native bar's menus (SwiftUI offers no
/// way to open a `Menu` from code) and the home screen. At each state it
/// prints `SHOT <name>` and holds still, so a host script watching the
/// `xcodebuild test` output can take a simulator screenshot (`simctl io
/// screenshot`); each is also kept as a test attachment.
///
/// Run with `./build.sh uitest` (see README.md).
final class NativeBarUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = true
    }

    /// The deep page with a long title (`-mbrSelfTest deep`): mbr's header is
    /// hidden, the title is in the bar, the three menus open, and `+` (New
    /// Note, beside ⋯ in one capsule) opens the new-note picker. The sample
    /// vault edits with no token, so `+` is there.
    func testMenusOnADeepPage() {
        let app = XCUIApplication()
        app.launchArguments += ["-mbrSelfTest", "deep"]
        app.launch()

        let more = app.navigationBars.buttons["More"]
        XCTAssertTrue(more.waitForExistence(timeout: 60), "the ⋯ menu")
        let newNote = app.navigationBars.buttons["New Note"]
        XCTAssertTrue(newNote.waitForExistence(timeout: 10), "the + (New Note) beside ⋯")
        let title = app.navigationBars.buttons
            .matching(NSPredicate(format: "label CONTAINS 'Roadmap'"))
            .firstMatch
        XCTAssertTrue(title.waitForExistence(timeout: 30), "the page title in the bar, as the breadcrumb menu")
        self.shot("deep")

        title.tap()
        XCTAssertTrue(app.buttons["Quarterly Planning"].waitForExistence(timeout: 5), "breadcrumb menu")
        self.shot("title-menu")
        self.dismissMenu(app)

        more.tap()
        XCTAssertTrue(app.buttons["Open Other Vault…"].waitForExistence(timeout: 5), "⋯ menu")
        XCTAssertTrue(app.buttons["Search"].exists, "the page's actions in the ⋯ menu")
        self.shot("more-menu")
        app.buttons["Search"].tap()
        let searchField = app.searchFields.firstMatch
        XCTAssertTrue(searchField.waitForExistence(timeout: 5), "the native search sheet")
        self.shot("search-from-menu")
        // The active field's own X, which closes the sheet (its Close is under it).
        app.buttons["Close"].firstMatch.tap()
        XCTAssertTrue(searchField.waitForNonExistence(timeout: 5), "the search sheet closed")

        let back = app.navigationBars.buttons["Back"]
        XCTAssertTrue(back.exists, "history Back")
        back.press(forDuration: 1.2)
        self.shot("back-menu")
        self.dismissMenu(app)

        // Last: the picker it opens is the page's, and the test ends there.
        newNote.tap()
        self.shot("new-note")
    }

    /// ⋯ → Search opens the native search sheet with the keyboard already up
    /// (the page's own modal could not raise it: WebKit starts no input
    /// session for a focus made from a native menu action). Typing queries the
    /// vault as you go; a result opens its page and closes the sheet.
    func testNativeSearchFromMenu() {
        let app = XCUIApplication()
        app.launchArguments += ["-mbrSelfTest", "deep"]
        app.launch()

        let more = app.navigationBars.buttons["More"]
        XCTAssertTrue(more.waitForExistence(timeout: 60), "the ⋯ menu")
        // The bar's actions arrive with the page's first bridge post.
        XCTAssertTrue(
            app.navigationBars.buttons.matching(NSPredicate(format: "label CONTAINS 'Roadmap'")).firstMatch
                .waitForExistence(timeout: 30),
            "the page has loaded"
        )
        more.tap()
        let search = app.buttons["Search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5), "Search in the ⋯ menu")
        search.tap()

        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5), "the native search field")
        XCTAssertTrue(
            app.keyboards.firstMatch.waitForExistence(timeout: 3),
            "the software keyboard is up at once after ⋯ → Search"
        )
        XCTAssertFalse(app.webViews.searchFields.firstMatch.exists, "the page's own modal stayed closed")

        app.typeText("flashcards")
        let result = app.buttons
            .matching(identifier: "searchResult")
            .matching(NSPredicate(format: "label BEGINSWITH 'Flashcards'"))
            .firstMatch
        XCTAssertTrue(result.waitForExistence(timeout: 10), "a result for the sample vault's Flashcards page")
        self.shot("native-search")
        guard result.exists else { return }

        result.tap()
        XCTAssertTrue(field.waitForNonExistence(timeout: 5), "the sheet closed")
        let title = app.navigationBars.buttons
            .matching(NSPredicate(format: "label CONTAINS 'Flashcards'"))
            .firstMatch
        XCTAssertTrue(title.waitForExistence(timeout: 15), "the Flashcards page is on screen")
        self.shot("native-search-opened")
    }

    /// A swipe in from the left edge opens Browse and one from the right opens
    /// Info (WebKit's back/forward swipes are off), in portrait and in
    /// landscape, where the edges are the screen's, beside the notch.
    func testEdgeSwipesOpenBrowseAndInfo() {
        for orientation in [UIDeviceOrientation.portrait, .landscapeLeft] {
            let suffix = orientation == .portrait ? "" : "-landscape"
            XCUIDevice.shared.orientation = orientation
            let app = XCUIApplication()
            app.launchArguments += ["-mbrSelfTest", "deep"]
            app.launch()
            // The actions (and so the swipes) arrive with the page's bridge post.
            let title = app.navigationBars.buttons
                .matching(NSPredicate(format: "label CONTAINS 'Roadmap'"))
                .firstMatch
            XCTAssertTrue(title.waitForExistence(timeout: 60), "the page has loaded\(suffix)")
            sleep(1)

            self.edgeSwipe(app, fromRight: false)
            // theme.css draws the heading uppercased, and WebKit reports it so.
            let navigate = app.webViews.staticTexts
                .matching(NSPredicate(format: "label ==[c] 'Navigate'"))
                .firstMatch
            XCTAssertTrue(navigate.waitForExistence(timeout: 5), "a left-edge swipe opened Browse\(suffix)")
            XCTAssertTrue(title.exists, "and did not go back a page\(suffix)")
            self.shot("swipe-browse\(suffix)")
            app.webViews.buttons["Close"].firstMatch.tap()
            XCTAssertTrue(navigate.waitForNonExistence(timeout: 5), "Browse closed\(suffix)")

            self.edgeSwipe(app, fromRight: true)
            let info = app.webViews.buttons["Close info panel"]
            XCTAssertTrue(info.waitForExistence(timeout: 5), "a right-edge swipe opened Info\(suffix)")
            self.shot("swipe-info\(suffix)")
            app.terminate()
        }
        XCUIDevice.shared.orientation = .portrait
    }

    /// The app icon on the home screen.
    func testHomeScreenIcon() {
        let app = XCUIApplication()
        app.launch()
        XCUIDevice.shared.press(.home)
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let icon = springboard.icons["MBR"]
        XCTAssertTrue(icon.waitForExistence(timeout: 10))
        // At most two pages: on an iPad a third swipe lands in the App Library
        // (which shows the icon too, but is not the home screen).
        for _ in 0 ..< 2 where !icon.isHittable {
            springboard.swipeLeft()
            sleep(1)
        }
        XCTAssertTrue(icon.exists, "MBR's icon is on the home screen")
        self.shot("home-icon")
    }

    private func shot(_ name: String) {
        sleep(2)
        print("SHOT \(name)")
        fflush(stdout)
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        self.add(attachment)
        sleep(4)
    }

    /// A drag in from 2pt inside one screen edge, half way down.
    private func edgeSwipe(_ app: XCUIApplication, fromRight: Bool) {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: fromRight ? 1 : 0, dy: 0.5))
            .withOffset(CGVector(dx: fromRight ? -2 : 2, dy: 0))
        let end = start.withOffset(CGVector(dx: fromRight ? -180 : 180, dy: 0))
        start.press(forDuration: 0.05, thenDragTo: end)
    }

    /// A tap low on the page, which the deep page leaves empty. (The status
    /// bar is no good: a tap there scrolls to top and leaves the menu open.)
    private func dismissMenu(_ app: XCUIApplication) {
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85)).tap()
        sleep(1)
    }
}
