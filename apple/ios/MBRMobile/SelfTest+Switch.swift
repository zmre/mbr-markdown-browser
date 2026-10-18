import Foundation

// MARK: switch: a document handed to the app while a vault is open

/// The share sheet ("Share → MBR") or "Open in" while a vault is on screen
/// replaces the path's one route with `.file(url)`. The screen at that
/// position must be a new one: a reused screen kept the vault's session and
/// never opened the file (device, 2026-10-16). Then "Grant Folder Access"
/// (`.file` → `.vault(start:)`, the same replacement) must open the folder at
/// that note, and Back must reach the vault list with no session left open.
extension SelfTest {
    /// The steps already reached; each `sessionStarted` advances one.
    private static var switchStep = 0
    static let switchFolder = "Switch Test"
    static let switchNote = "switch-doc.md"
    /// Seconds a step may take before the next session counts as never
    /// opened.
    private static let switchTimeout: Double = 20

    static var switchDocument: URL {
        AppPaths.documents
            .appending(path: self.switchFolder, directoryHint: .isDirectory)
            .appending(path: self.switchNote)
    }

    /// Called for every session the scenario opens. Unstructured, because
    /// each step replaces the screen whose `.task` called it.
    static func runSwitch(_ session: VaultSession, model: AppModel) {
        self.switchStep += 1
        let step = self.switchStep
        Task {
            switch step {
            case 1: await self.switchVaultOpened(session, model: model)
            case 2: await self.switchDocumentOpened(session, model: model)
            case 3: await self.switchGrantedFolderOpened(session, model: model)
            default: self.log("switch", "FAIL unexpected session #\(step): \(session.title)")
            }
        }
    }

    /// Step 1: the sample vault is on screen. Hand the app a document.
    private static func switchVaultOpened(_ session: VaultSession, model: AppModel) async {
        let document = self.switchDocument
        await Task.detached {
            let folder = document.deletingLastPathComponent()
            try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try? "# Switched document\n\nOpened while a vault was open.\n"
                .write(to: document, atomically: true, encoding: .utf8)
        }.value
        self.log("switch.vault", "\(session.title) open; handing the app \(document.lastPathComponent)")
        model.openDocument(document)
        self.expectStep(2, "the document did not open over the vault")
    }

    /// Step 2: the document replaced the vault. Grant its folder.
    private static func switchDocumentOpened(_ session: VaultSession, model: AppModel) async {
        let current = model.session === session
        let page = await self.js(session, """
        const h1 = document.querySelector('h1')?.textContent ?? null;
        return `location=${location.pathname} h1=${JSON.stringify(h1)}`;
        """)
        self.log("switch.document", page.withVerdict(
            session.isSingleFile && current,
            "singleFile=\(session.isSingleFile) current=\(current)"
        ))
        let folder = self.switchDocument.deletingLastPathComponent()
        let vault = model.store.addContainerFolder(self.switchFolder, name: self.switchFolder)
        let start = VaultScreen.pagePath(of: self.switchDocument, in: folder)
        self.log("switch.grant", "folder becomes a vault, opening at \(start ?? "nil")")
        // What VaultScreen.picked does for a file.
        model.path = [.vault(vault.id, start: start)]
        self.expectStep(3, "the granted folder did not open")
    }

    /// Step 3: the granted folder opened at the note. Back to the list.
    private static func switchGrantedFolderOpened(_ session: VaultSession, model: AppModel) async {
        let current = model.session === session
        let page = await self.js(session, """
        const h1 = document.querySelector('h1')?.textContent ?? null;
        return `location=${location.pathname} h1=${JSON.stringify(h1)}`;
        """)
        let atNote = page.contains("location=/switch-doc/")
        self.log("switch.granted", page.withVerdict(
            !session.isSingleFile && current && atNote,
            "singleFile=\(session.isSingleFile) current=\(current) atNote=\(atNote)"
        ))
        // What Back does at the first page.
        model.path = []
        try? await Task.sleep(for: .seconds(2))
        let closed = model.session == nil
        self.log("switch.back", "\(closed ? "PASS" : "FAIL") list shown, session closed=\(closed)")
        self.log("done", "switch")
    }

    /// Fails the scenario when step `step` has not been reached in time.
    private static func expectStep(_ step: Int, _ failure: String) {
        Task {
            try? await Task.sleep(for: .seconds(self.switchTimeout))
            if self.switchStep < step {
                self.log("switch", "FAIL \(failure) within \(Int(self.switchTimeout))s")
                self.log("done", "switch")
            }
        }
    }
}

private extension String {
    /// `PASS <self> (<detail>)` when `passed`, else `FAIL …`.
    func withVerdict(_ passed: Bool, _ detail: String) -> String {
        "\(passed ? "PASS" : "FAIL") \(self) (\(detail))"
    }
}
