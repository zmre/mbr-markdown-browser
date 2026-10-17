import Foundation

// MARK: full: the pre-refresh provider request

extension SelfTest {
    /// `ProviderRefresh` on the sample vault. The simulator has no file
    /// provider, so this checks the mechanics, not a provider's answer: the
    /// location test, the folder choice, that presenters and the coordinated
    /// read complete and come off again, that an idle run costs its budget
    /// and no more, and that a change made during the wait ends it early.
    static func providerRefresh(_ session: VaultSession) async -> String {
        let root = session.access.url
        let folder = root.appending(path: self.fixtureFolder, directoryHint: .isDirectory)
        let note = folder.appending(path: "tasks.md")
        let probe = folder.appending(path: "provider-probe.txt")
        var problems: [String] = []

        if ProviderRefresh.isProviderLocation(root.path(percentEncoded: false)) {
            problems.append("sample vault taken for provider storage")
        }
        if !ProviderRefresh.isProviderLocation("/private/var/mobile/Library/CloudStorage/A1B2/Dev/docs") {
            problems.append("CloudStorage not recognised")
        }
        let chosen = ProviderRefresh.folder(root: root, file: note, listing: nil, singleFile: false)
        if chosen.standardizedFileURL.path() != folder.standardizedFileURL.path() {
            problems.append("folder=\(chosen.path())")
        }

        let (idle, early) = await Task.detached {
            try? "before\n".write(to: probe, atomically: true, encoding: .utf8)
            let idle = ProviderRefresh.refresh(folder: folder, file: probe, budget: .milliseconds(600))
            // A write 300 ms into the wait, as a provider updating its replica.
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.3) {
                try? "after, longer\n".write(to: probe, atomically: true, encoding: .utf8)
            }
            let early = ProviderRefresh.refresh(folder: folder, file: probe, budget: .seconds(3))
            try? FileManager.default.removeItem(at: probe)
            return (idle, early)
        }.value

        if idle.fileChanged || idle.read == "timed out" || idle.millis < 550 || idle.millis > 1500 {
            problems.append("idle: \(idle.millis)ms changed=\(idle.fileChanged) read=\(idle.read)")
        }
        if !early.fileChanged || early.millis > 2000 {
            problems.append("early: \(early.millis)ms changed=\(early.fileChanged)")
        }
        return (problems.isEmpty ? "PASS" : "FAIL \(problems.joined(separator: "; "))")
            + " idle=\(idle.millis)ms read=\(idle.read) early=\(early.millis)ms "
            + "signal=\(early.signal ?? "none")"
    }
}
