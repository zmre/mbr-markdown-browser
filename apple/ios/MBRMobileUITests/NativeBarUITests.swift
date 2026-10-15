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
    /// hidden, the title is in the bar, and the three menus open.
    func testMenusOnADeepPage() {
        let app = XCUIApplication()
        app.launchArguments += ["-mbrSelfTest", "deep"]
        app.launch()

        let more = app.navigationBars.buttons["More"]
        XCTAssertTrue(more.waitForExistence(timeout: 60), "the ⋯ menu")
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
        self.shot("search-from-menu")
        self.dismissMenu(app)

        let back = app.navigationBars.buttons["Back"]
        XCTAssertTrue(back.exists, "history Back")
        back.press(forDuration: 1.2)
        self.shot("back-menu")
        self.dismissMenu(app)
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

    /// A tap low on the page, which the deep page leaves empty. (The status
    /// bar is no good: a tap there scrolls to top and leaves the menu open.)
    private func dismissMenu(_ app: XCUIApplication) {
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85)).tap()
        sleep(1)
    }
}
