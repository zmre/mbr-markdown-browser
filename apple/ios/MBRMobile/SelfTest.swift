import Foundation
import WebKit

/// Simulator verification, driven by launch arguments. Every check goes
/// through the real web view, the real `mbr://` scheme handler and the real
/// `FileCoordinator`; results are logged as `SELFTEST <name>: <result>`.
///
/// | Launch argument | What runs |
/// |-----------------|-----------|
/// | `-mbrSelfTest full` (or `YES`) | the sample vault: rendering, chunks, search, writes, media, redirect |
/// | `-mbrSelfTest single -mbrOpenFile <path under Documents>` | a single file opened in place |
/// | `-mbrSelfTest config` | a vault whose `.mbr/config.toml` is broken |
/// | `-mbrSelfTest refresh` | opens the sample vault and waits (the host edits files while backgrounded) |
/// | `-mbrSelfTest download` | a legacy `.Name.md.icloud` placeholder: the Downloading page and its retries |
/// | `-mbrSelfTest screens` | steps through screens, logging `SCREEN <name>` for screenshots |
///
/// A file opened with `simctl openurl` while `single` is active is checked
/// the same way as one passed with `-mbrOpenFile`.
@MainActor
enum SelfTest {
    static var mode: String? {
        guard let raw = UserDefaults.standard.string(forKey: "mbrSelfTest") else { return nil }
        return raw == "YES" ? "full" : raw
    }

    static var isActive: Bool {
        mode != nil
    }

    static let fixtureFolder = "selftest"
    nonisolated static let taskLine = "- [ ] toggle me from the web view"

    // MARK: Entry points

    /// At launch: open whatever the mode tests.
    static func startIfRequested(_ model: AppModel) async {
        guard let mode else { return }
        self.log("start", "mode=\(mode)")
        switch mode {
        case "single":
            if let relative = UserDefaults.standard.string(forKey: "mbrOpenFile") {
                model.openDocument(AppPaths.documents.appending(path: relative))
            } else {
                self.log("start", "waiting for a document (simctl openurl)")
            }
        case "config":
            let folder = "Broken Config"
            await writeBrokenConfigVault(folder)
            let vault = model.store.addContainerFolder(folder, name: "Broken Config")
            model.path = [.vault(vault.id)]
        default:
            if let sample = model.store.vaults.first(where: { $0.location == .container(AppPaths.sampleFolder) }) {
                model.path = [.vault(sample.id)]
            }
        }
    }

    /// A session finished loading its first page.
    static func sessionStarted(_ session: VaultSession, model: AppModel) async {
        guard let mode else { return }
        switch mode {
        case "full": await self.runFull(session)
        case "single": await self.runSingle(session)
        case "config": await self.runConfig(session)
        case "download": await self.runDownload(session)
        case "deep": await self.runDeep(session)
        case "mbr-reload": await self.runMbrReload(session)
        case "screens":
            // Unstructured: the sequence leaves the vault screen, which
            // cancels the task that called this.
            Task { await self.runScreens(session, model: model) }
        case "refresh":
            self.log("refresh", "waiting; background the app, edit a file, foreground it")
        default:
            self.log("start", "unknown mode \(mode)")
        }
    }

    // MARK: full

    /// In-page checks on the vault's home page: (name, function body).
    static let pageChecks: [(String, String)] = [
        ("viewport", """
        const desktop = matchMedia('(min-width: 1024px)').matches;
        return `${innerWidth}x${innerHeight} desktopLayout=${desktop}`;
        """),
        // Persists across launches if WebKit's data store does: the second
        // launch reports the first launch's timestamp.
        ("localStorage", """
        const previous = localStorage.getItem('mbr-selftest-launch');
        localStorage.setItem('mbr-selftest-launch', new Date().toISOString());
        return previous === null ? 'none (first launch)' : `previous launch ${previous}`;
        """),
        ("components", """
        const names = ['mbr-browse', 'mbr-search', 'mbr-info', 'mbr-mermaid'];
        return names.map(n => `${n}=${customElements.get(n) ? 'defined' : 'missing'}`).join(' ');
        """),
        ("liveReload", """
        return `mbr-live-reload ${document.querySelector('mbr-live-reload') ? 'present' : 'absent'}, `
          + `protocol ${location.protocol} (no socket on a custom scheme)`;
        """),
        ("mermaid", """
        for (let i = 0; i < 75; i++) {
          const svgs = document.querySelectorAll('pre.mermaid svg, div.mermaid svg, mbr-mermaid svg').length;
          if (svgs > 0) return `PASS rendered ${svgs} diagram(s)`;
          await new Promise(r => setTimeout(r, 200));
        }
        const blocks = document.querySelectorAll('pre.mermaid, div.mermaid, code.language-mermaid').length;
        return `FAIL not rendered (blocks=${blocks})`;
        """),
        ("lazyChunk", """
        const t0 = performance.now();
        try {
          const m = await import('/.mbr/components/mbr-graph.min.js');
          const ms = Math.round(performance.now() - t0);
          const defined = customElements.get('mbr-mini-graph') ? 'defined' : 'missing';
          return `PASS import() in ${ms}ms (${Object.keys(m).length} exports), mbr-mini-graph=${defined}`;
        } catch (e) {
          return `FAIL import(): ${e}`;
        }
        """),
        ("search", """
        const body = JSON.stringify({ q: 'mermaid', scope: 'all', limit: 5 });
        const t0 = performance.now();
        const r = await fetch('/.mbr/search', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body });
        const ms = Math.round(performance.now() - t0);
        if (!r.ok) return `FAIL HTTP ${r.status} ${await r.text()}`;
        const j = await r.json();
        return `${j.results.length > 0 ? 'PASS' : 'FAIL'} HTTP ${r.status}, ${j.results.length} results of ${j.total_matches} in ${ms}ms`;
        """),
        ("range", """
        const r = await fetch('/videos/demo.mp4', { headers: { Range: 'bytes=0-99' } });
        const n = (await r.arrayBuffer()).byteLength;
        const ok = r.status === 206 && n === 100;
        return `${ok ? 'PASS' : 'FAIL'} status=${r.status} bytes=${n} content-range=${r.headers.get('content-range')}`;
        """)
    ]

    static func runFull(_ session: VaultSession) async {
        var results: [(String, String)] = []
        func record(_ name: String, _ value: String) {
            results.append((name, value))
            self.log(name, value)
        }

        let report = try? await session.app.waitUntilReady()
        record(
            "open",
            "fromCache=\(report?.fromCache.description ?? "?") open=\(session.openMs)ms "
                + "scan=\(report?.scanMs ?? 0)ms ready=\(report?.readyMs ?? 0)ms "
                + "firstPage=\(session.firstPageMs.map(String.init) ?? "?")ms indexed=\(report?.indexedFiles ?? 0)"
        )

        await self.writeFixtures(session)

        for (name, body) in self.pageChecks {
            await record(name, self.js(session, body))
        }
        let info = session.web.info
        let bar = "title=\(info.title) crumbs=\(info.breadcrumbs.count) actions=\(info.actions.map(\.rawValue))"
        record("nativeBar", (info.title.isEmpty ? "FAIL " : "PASS ") + bar)
        let opened = await AppBridge.open(.search, in: session.page)
        await record("bridgeOpen", (opened ? "PASS" : "FAIL") + " search -> \(opened); " + self.js(session, """
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
        const h = document.querySelector('body > header');
        return `embedded=${window.__MBR_EMBEDDED__ === true} headerHeight=${h?.getBoundingClientRect().height}`;
        """))

        await record("taskToggle", self.taskToggle(session))
        await record("flashcardReview", self.flashcardReview(session))
        await record("videoSeek", self.videoSeek(session))
        await record("providerRefresh", self.providerRefresh(session))

        // A slashless URL: mbr answers 301 to /modes/, which WebKit would not
        // follow from a scheme handler; the transport replays it in the page.
        await session.load(URL(string: "mbr://localhost/modes")!, label: "redirect")
        try? await Task.sleep(for: .milliseconds(500))
        await record("redirect", self.js(session, """
        const ok = location.pathname === '/modes/';
        return `${ok ? 'PASS' : 'FAIL'} location=${location.href} h1=${JSON.stringify(document.querySelector('h1')?.textContent ?? null)}`;
        """))

        await session.load(MbrSchemeHandler.origin, label: "home again")
        self.log("done", results.map { "\($0.0)=\($0.1)" }.joined(separator: " | "))
    }

    // MARK: Helpers

    /// The fixture notes, written into the sample vault and picked up with a
    /// refresh (which exercises `refresh()` too).
    static func writeFixtures(_ session: VaultSession) async {
        let folder = session.access.url.appending(path: self.fixtureFolder, directoryHint: .isDirectory)
        await Task.detached {
            let fileManager = FileManager.default
            try? fileManager.createDirectory(at: folder, withIntermediateDirectories: true)
            let notes = [
                "tasks.md": "# Self-test tasks\n\n\(taskLine)\n",
                "cards.md": "---\ntype: flashcard\n---\n\n# Self-test cards\n\nWhat is mbr?\n: A markdown browser.\n",
                "media.md": "# Self-test media\n\n![Demo video](../videos/demo.mp4)\n"
            ]
            for (name, text) in notes {
                try? text.write(to: folder.appending(path: name), atomically: true, encoding: .utf8)
            }
        }.value
        await session.refresh(reason: "selftest fixtures")
    }

    static func poll(_ file: URL, until done: @escaping @Sendable (String) -> Bool) async -> String {
        var text = ""
        for _ in 0 ..< 40 {
            try? await Task.sleep(for: .milliseconds(100))
            text = await Task.detached { (try? String(contentsOf: file, encoding: .utf8)) ?? "" }.value
            if done(text) { break }
        }
        return text
    }

    static func js(_ session: VaultSession, _ body: String) async -> String {
        do {
            let value = try await session.page.callJavaScript(body)
            return value.map { "\($0)" } ?? "nil"
        } catch {
            return "JS error: \(error)"
        }
    }

    static func log(_ name: String, _ value: String) {
        Log.info("SELFTEST \(name): \(value)")
    }
}
