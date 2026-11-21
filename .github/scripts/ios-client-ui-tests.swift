// Compiled only in the disposable host-side XcodeGen project, never in the product.
// https://developer.apple.com/documentation/xcuiautomation/xcuiapplication/init(bundleidentifier:)
import XCTest

final class ClientAcceptanceTests: XCTestCase {
    @MainActor
    func testOfflineRestartAllowsAddressChangeAndReconnect() throws {
        continueAfterFailure = false
        executionTimeAllowance = 180
        let env = ProcessInfo.processInfo.environment
        let app = XCUIApplication(bundleIdentifier: try XCTUnwrap(env["CLIENT_ACCEPTANCE_BUNDLE_ID"]))
        let previousURL = try XCTUnwrap(env["CLIENT_ACCEPTANCE_URL_A"])
        let nextURL = try XCTUnwrap(env["CLIENT_ACCEPTANCE_URL_B"])
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        defer { attach(app, "offline-reconnect-final") }
        app.launch()
        let address = app.webViews.textFields["Server address"].firstMatch
        let open = app.webViews.buttons["Open workspace"].firstMatch
        XCTAssertTrue(address.waitForExistence(timeout: 45))
        let error = app.webViews.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Cannot connect to the server.")).firstMatch
        XCTAssertTrue(error.waitForExistence(timeout: 40))
        XCTAssertEqual(normalize(address.value as? String ?? ""), normalize(previousURL))
        replaceAddress(app, address: address, with: nextURL)
        assertKeyboardReachability(app, address: address, open: open)
        attach(app, "offline-address-changed-with-keyboard")
        open.tap()
        waitForFixture(app)
        settings(app)
        XCTAssertEqual(normalize(address.value as? String ?? ""), normalize(nextURL))
    }

    @MainActor
    func testFirstConfigurationKeyboardServerChangeAndRestart() throws {
        continueAfterFailure = false
        executionTimeAllowance = 180
        let env = ProcessInfo.processInfo.environment
        let identifier = try XCTUnwrap(env["CLIENT_ACCEPTANCE_BUNDLE_ID"])
        let firstURL = try XCTUnwrap(env["CLIENT_ACCEPTANCE_URL_A"])
        let secondURL = try XCTUnwrap(env["CLIENT_ACCEPTANCE_URL_B"])
        // The scheme's app is a disposable host, not this identifier. Apple documents
        // that an existing installed app is launched when no matching build exists.
        let app = XCUIApplication(bundleIdentifier: identifier)
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        defer {
            let screenshot = XCTAttachment(screenshot: app.screenshot())
            screenshot.name = "final-native-ui"
            screenshot.lifetime = .keepAlways
            add(screenshot)
            let tree = XCTAttachment(string: app.debugDescription)
            tree.name = "final-accessibility-tree"
            tree.lifetime = .keepAlways
            add(tree)
        }
        app.launch()
        let address = app.webViews.textFields["Server address"].firstMatch
        let open = app.webViews.buttons["Open workspace"].firstMatch
        XCTAssertTrue(address.waitForExistence(timeout: 30))
        let initial = address.value as? String ?? ""
        XCTAssertTrue(initial.isEmpty || initial == "https://", "Fresh input has a configured server: \(initial)")
        XCTAssertFalse(app.buttons["Start service"].exists)
        address.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10), "Actual software keyboard must be shown")
        address.typeText(firstURL)
        assertKeyboardReachability(app, address: address, open: open)
        attach(app, "first-address-keyboard")
        open.tap()
        waitForFixture(app)
        settings(app)
        XCTAssertEqual(normalize(address.value as? String ?? ""), normalize(firstURL))
        replaceAddress(app, address: address, with: secondURL)
        assertKeyboardReachability(app, address: address, open: open)
        attach(app, "changed-address-keyboard")
        open.tap()
        waitForFixture(app)
        app.terminate()
        app.launch()
        waitForFixture(app)
        settings(app)
        XCTAssertEqual(normalize(address.value as? String ?? ""), normalize(secondURL), "User's changed URL did not survive process restart")
        XCTAssertTrue(address.isHittable)
        attach(app, "changed-address-restored")
    }

    private func normalize(_ url: String) -> String {
        url.hasSuffix("/") ? String(url.dropLast()) : url
    }

    @MainActor
    private func settings(_ app: XCUIApplication) {
        let link = app.webViews.links["Connection settings"].firstMatch
        XCTAssertTrue(link.waitForExistence(timeout: 30))
        link.tap()
        XCTAssertTrue(app.webViews.textFields["Server address"].firstMatch.waitForExistence(timeout: 30))
    }

    @MainActor
    private func waitForFixture(_ app: XCUIApplication) {
        XCTAssertTrue(app.webViews.staticTexts["NATIVE FIXTURE READY"].firstMatch.waitForExistence(timeout: 40))
        XCTAssertTrue(app.webViews.staticTexts["COOKIE VERIFIED"].firstMatch.waitForExistence(timeout: 15))
    }

    @MainActor
    private func replaceAddress(_ app: XCUIApplication, address: XCUIElement, with value: String) {
        let previous = address.value as? String ?? ""
        address.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        // Fixture URLs are short and fully visible; right-edge tap places the caret
        // after the existing value, then ordinary Delete key events clear the field.
        address.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
        address.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: previous.count + 1))
        let empty = address.value as? String ?? ""
        XCTAssertTrue(empty.isEmpty || empty == "https://", "Input did not clear through actual keyboard events")
        address.typeText(value)
    }

    @MainActor
    private func assertKeyboardReachability(_ app: XCUIApplication, address: XCUIElement, open: XCUIElement) {
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.exists)
        XCTAssertTrue(address.isHittable)
        XCTAssertTrue(open.isHittable, "Core action must remain hittable with the software keyboard open")
        XCTAssertGreaterThan(address.frame.width, 0)
        XCTAssertGreaterThan(open.frame.height, 0)
        XCTAssertGreaterThanOrEqual(address.frame.minX, app.frame.minX)
        XCTAssertLessThanOrEqual(address.frame.maxX, app.frame.maxX)
        XCTAssertGreaterThanOrEqual(open.frame.minX, app.frame.minX)
        XCTAssertLessThanOrEqual(open.frame.maxX, app.frame.maxX)
        XCTAssertLessThanOrEqual(address.frame.maxY, keyboard.frame.minY + 1)
        XCTAssertLessThanOrEqual(open.frame.maxY, keyboard.frame.minY + 1, "Open workspace overlaps the keyboard")
    }

    @MainActor
    private func attach(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
