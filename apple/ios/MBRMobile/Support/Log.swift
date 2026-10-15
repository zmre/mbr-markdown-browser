import Foundation
import os

/// The app's log: the unified log (Console.app, filter on subsystem
/// `com.zmre.mbr.ios`) plus stderr, which is unbuffered and so shows up at
/// once under `simctl launch --console-pty`.
///
/// Every line is `.public`: these are the device-measurement lines
/// (`open:`, `scan:`, `ready:`, `first page:`, every request) that TESTING.md
/// asks to read back, on the owner's own device.
enum Log {
    private static let logger = Logger(subsystem: "com.zmre.mbr.ios", category: "app")

    static func info(_ message: String) {
        self.logger.notice("\(message, privacy: .public)")
        FileHandle.standardError.write(Data("[mbr] \(message)\n".utf8))
    }
}

/// Milliseconds elapsed since `start`.
func millis(since start: ContinuousClock.Instant) -> Int {
    let elapsed = ContinuousClock.now - start
    return Int(elapsed.components.seconds * 1000)
        + Int(elapsed.components.attoseconds / 1_000_000_000_000_000)
}
