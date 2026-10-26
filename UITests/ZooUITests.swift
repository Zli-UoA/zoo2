import XCTest

final class ZooUITests: XCTestCase {
    @MainActor
    func testWelcomeHasAccessibleLogin() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting"]
        app.launch()
        XCTAssertTrue(app.buttons["login"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["login"].isEnabled)
    }
}
