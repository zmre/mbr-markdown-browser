import Foundation
import WebKit

/// The self-test's individual scenarios. See `SelfTest`.
extension SelfTest {
    /// The task-test page, toggled through its real checkbox (task-toggle.ts:
    /// `GET /.mbr/raw`, then `POST /.mbr/task`), checked on disk.
    static func taskToggle(_ session: VaultSession) async -> String {
        let file = session.access.url.appending(path: "\(self.fixtureFolder)/tasks.md")
        let writesBefore = session.coordinator.count(of: "write")
        await session.load(URL(string: "mbr://localhost/\(self.fixtureFolder)/tasks/")!, label: "task page")
        let clicked = await js(session, """
        const box = document.querySelector('input.mbr-task-check');
        if (!box) return 'no checkbox on page';
        box.click();
        return `clicked (editEnabled=${window.__MBR_CONFIG__?.editEnabled})`;
        """)
        let onDisk = await poll(file) { $0.contains("- [x]") }
        let writes = session.coordinator.count(of: "write") - writesBefore
        let line = onDisk.split(separator: "\n").first { $0.contains("toggle me") }.map(String.init) ?? ""
        let pass = onDisk.contains("- [x]") && writes > 0
        return "\(pass ? "PASS" : "FAIL") \(clicked); coordinator writes=\(writes); file: \(line)"
    }

    /// Opens the deck on the card page, flips the card and rates it Good;
    /// the deck's writer appends to the card's review history.
    static func flashcardReview(_ session: VaultSession) async -> String {
        let file = session.access.url.appending(path: "\(self.fixtureFolder)/cards.md")
        let writesBefore = session.coordinator.count(of: "write")
        await session.load(URL(string: "mbr://localhost/\(self.fixtureFolder)/cards/")!, label: "card page")
        let driven = await js(session, """
        const sleep = ms => new Promise(r => setTimeout(r, ms));
        for (let i = 0; i < 50 && !customElements.get('mbr-flashcards'); i++) await sleep(100);
        const trigger = document.querySelector('mbr-flashcards');
        if (!trigger) return 'no <mbr-flashcards> on page';
        trigger.open();
        let deck = null;
        for (let i = 0; i < 50 && !(deck = document.querySelector('mbr-flashcard-deck')); i++) await sleep(100);
        if (!deck) return 'deck did not open';
        await sleep(600);
        document.activeElement?.blur?.();
        const key = k => window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
        key('Enter');
        await sleep(400);
        const good = [...deck.querySelectorAll('button')].find(b => /good/i.test(b.textContent ?? ''));
        if (good) { good.click(); } else { key('3'); }
        await sleep(300);
        return `deck open, rated Good via ${good ? 'button' : 'key 3'}`;
        """)
        let onDisk = await poll(file) { $0.contains("- Good") }
        let writes = session.coordinator.count(of: "write") - writesBefore
        let entry = onDisk.split(separator: "\n").last { $0.contains("Good") }.map(String.init) ?? ""
        let pass = onDisk.contains("- Good") && writes > 0
        return "\(pass ? "PASS" : "FAIL") \(driven); coordinator writes=\(writes); history: \(entry)"
    }

    /// A `<video>` over `mbr://` must load metadata and seek (Range/206).
    static func videoSeek(_ session: VaultSession) async -> String {
        await session.load(URL(string: "mbr://localhost/\(self.fixtureFolder)/media/")!, label: "media page")
        return await self.js(session, """
        const v = document.querySelector('video');
        if (!v) return 'FAIL no <video> on page';
        v.muted = true;
        const wait = (event, ms) => new Promise((resolve, reject) => {
          v.addEventListener(event, resolve, { once: true });
          v.addEventListener('error', () => reject(`media error ${v.error?.code} ${v.error?.message ?? ''}`), { once: true });
          setTimeout(() => reject(`timeout waiting for ${event}`), ms);
        });
        try {
          if (v.readyState < 1) { v.preload = 'auto'; if (v.networkState === 3 || v.networkState === 0) v.load(); await wait('loadedmetadata', 15000); }
          const target = Math.min(3, v.duration / 2);
          const seeked = wait('seeked', 15000);
          v.currentTime = target;
          await seeked;
          return `PASS duration=${v.duration.toFixed(2)}s seeked to ${v.currentTime.toFixed(2)}s readyState=${v.readyState} src=${v.currentSrc}`;
        } catch (e) {
          return `FAIL ${e} readyState=${v.readyState} src=${v.currentSrc}`;
        }
        """)
    }

    // MARK: single

    static func runSingle(_ session: VaultSession) async {
        guard session.isSingleFile else {
            self.log("single", "FAIL session is not a single file")
            return
        }
        let home = session.app.homeUrl()
        await self.log("single.render", self.js(session, """
        const h1 = document.querySelector('h1')?.textContent ?? null;
        return `${h1 ? 'PASS' : 'FAIL'} location=${location.pathname} h1=${JSON.stringify(h1)}`;
        """) + " home=\(home)")
        await session.load(MbrSchemeHandler.origin, label: "single root")
        try? await Task.sleep(for: .milliseconds(500))
        await self.log("single.root", self.js(session, """
        return `location=${location.pathname}`;
        """) + " (expected \(home))")
        self.log("single.grant", "Grant Folder Access offered=\(session.isSingleFile) root=\(session.app.rootPath())")
        self.log("done", "single")
    }

    // MARK: config

    static func writeBrokenConfigVault(_ folder: String) async {
        let root = AppPaths.documents.appending(path: folder, directoryHint: .isDirectory)
        await Task.detached {
            let fileManager = FileManager.default
            try? fileManager.createDirectory(at: root.appending(path: ".mbr"), withIntermediateDirectories: true)
            try? "theme = \"amber\"\nthis is = = not toml\n".write(
                to: root.appending(path: ".mbr/config.toml"), atomically: true, encoding: .utf8
            )
            try? "# Broken config\n\nThe vault still opens.\n".write(
                to: root.appending(path: "index.md"), atomically: true, encoding: .utf8
            )
        }.value
    }

    static func runConfig(_ session: VaultSession) async {
        self.log("config.error", session.configError.map { "PASS \($0)" } ?? "FAIL no config error reported")
        await self.log("config.page", self.js(session, """
        const r = await fetch('/errors.json');
        const text = await r.text();
        return `errors.json HTTP ${r.status} config_parse_error=${text.includes('config_parse_error')} h1=${JSON.stringify(document.querySelector('h1')?.textContent ?? null)}`;
        """))
        self.log("done", "config")
    }

    // MARK: download

    /// A legacy iCloud placeholder (`.pending.md.icloud`) with no real file:
    /// the vault lists `pending.md` as not downloaded, and its page answers
    /// 503 "Downloading" with `Retry-After`/`Refresh`. After a while the
    /// "download" completes (the real file appears) and the page must render.
    static func runDownload(_ session: VaultSession) async {
        let folder = session.access.url.appending(path: self.fixtureFolder, directoryHint: .isDirectory)
        await Task.detached {
            try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try? FileManager.default.removeItem(at: folder.appending(path: "pending.md"))
            try? Data("bplist00 placeholder".utf8).write(to: folder.appending(path: ".pending.md.icloud"))
        }.value
        await session.refresh(reason: "selftest download")
        let url = URL(string: "mbr://localhost/\(fixtureFolder)/pending/")!
        await session.load(url, label: "downloading page")
        await self.log("download.page", self.js(session, """
        return `title=${JSON.stringify(document.title)} h1=${JSON.stringify(document.querySelector('h1')?.textContent ?? null)} name=${JSON.stringify(document.querySelector('.mbr-downloading-name')?.textContent ?? null)} progress=${!!document.querySelector('.mbr-downloading progress')}`;
        """))
        FileHandle.standardError.write(Data("SCREEN downloading\n".utf8))
        // What `<mbr-page-errors>` gets on each of those loads.
        await self.log("download.errorsJson", self.js(session, """
        const r = await fetch('/\(self.fixtureFolder)/pending/errors.json');
        return `HTTP ${r.status} ${r.headers.get('content-type')} ${await r.text()}`;
        """))
        try? await Task.sleep(for: .seconds(9))
        self.log(
            "download.refresh",
            "WebKit honours Refresh: "
                + (session.webKitHonoursRefresh.map { $0 ? "yes" : "no (Swift retried)" } ?? "undetermined")
        )
        await Task.detached {
            try? "# Arrived\n\nThe download finished.\n".write(
                to: folder.appending(path: "pending.md"), atomically: true, encoding: .utf8
            )
            try? FileManager.default.removeItem(at: folder.appending(path: ".pending.md.icloud"))
        }.value
        self.log("download", "real file written, placeholder removed; waiting for the page")
        var result = ""
        for _ in 0 ..< 20 {
            try? await Task.sleep(for: .milliseconds(500))
            result = await self.js(session, "return document.querySelector('h1')?.textContent ?? ''")
            if result.contains("Arrived") { break }
        }
        self.log("download.arrived", "\(result.contains("Arrived") ? "PASS" : "FAIL") h1=\(result)")
        self.log("done", "download")
    }

    // MARK: screens

    static func runScreens(_ session: VaultSession, model: AppModel) async {
        func screen(_ name: String, hold seconds: Double = 4) async {
            log("screen", name)
            FileHandle.standardError.write(Data("SCREEN \(name)\n".utf8))
            try? await Task.sleep(for: .seconds(seconds))
        }
        try? await Task.sleep(for: .seconds(2))
        await screen("page")
        _ = await self.js(session, """
        const s = document.querySelector('mbr-search');
        s.open();
        await new Promise(r => setTimeout(r, 600));
        const input = s.shadowRoot?.querySelector('input') ?? s.querySelector('input');
        if (input) { input.value = 'flashcards'; input.dispatchEvent(new Event('input', { bubbles: true, composed: true })); }
        await new Promise(r => setTimeout(r, 1200));
        return 'ok';
        """)
        await screen("search")
        _ = await self.js(session, """
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
        return 'ok';
        """)
        await session.load(URL(string: "mbr://localhost/markdown/flashcards/")!, label: "deck page")
        _ = await self.js(session, """
        for (let i = 0; i < 50 && !customElements.get('mbr-flashcards'); i++) await new Promise(r => setTimeout(r, 100));
        document.querySelector('mbr-flashcards')?.open();
        await new Promise(r => setTimeout(r, 1500));
        return 'ok';
        """)
        await screen("deck")
        _ = await self.js(session, """
        window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
        return 'ok';
        """)
        // The editor and the tasks overlay, opened the way the app's ⋯ menu
        // opens them: through the bridge.
        await session.load(MbrSchemeHandler.origin, label: "home")
        try? await Task.sleep(for: .seconds(1))
        await self.log("screen.edit", "bridge open -> \(AppBridge.open(.edit, in: session.page))")
        try? await Task.sleep(for: .seconds(4))
        await screen("editor")
        _ = await self.js(session, """
        document.querySelector('.mbr-editor-close')?.click();
        return 'ok';
        """)
        try? await Task.sleep(for: .seconds(1))
        await self.log("screen.tasks", "bridge open -> \(AppBridge.open(.tasks, in: session.page))")
        try? await Task.sleep(for: .seconds(3))
        await screen("tasks")
        _ = await self.js(session, """
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
        return 'ok';
        """)
        model.path = []
        try? await Task.sleep(for: .seconds(1))
        await screen("list")
        model.showSettings = true
        try? await Task.sleep(for: .seconds(2))
        await screen("settings")
        model.showSettings = false
        self.log("done", "screens")
    }

    // MARK: deep

    /// A page four folders down with a long title: what the native bar
    /// (title, breadcrumb menu, ⋯ menu, Back) is for. Stays open, logging
    /// `SCREEN deep`, for the UI test that opens the menus.
    static func runDeep(_ session: VaultSession) async {
        let folder = session.access.url.appending(
            path: "Projects/Clients/Acme Corporation/Quarterly Planning", directoryHint: .isDirectory
        )
        await Task.detached {
            try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try? """
            ---
            title: 2026 Q4 Roadmap, Budget Review and Hiring Plan for the Platform Team
            ---

            # 2026 Q4 Roadmap, Budget Review and Hiring Plan for the Platform Team

            A deep page with a long title.

            - [ ] Draft the budget
            - [ ] Review the hiring plan
            """.write(to: folder.appending(path: "Roadmap.md"), atomically: true, encoding: .utf8)
        }.value
        await session.refresh(reason: "selftest deep")
        let page = "mbr://localhost/Projects/Clients/Acme%20Corporation/Quarterly%20Planning/Roadmap/"
        await session.load(URL(string: page)!, label: "deep page")
        try? await Task.sleep(for: .seconds(1))
        let info = session.web.info
        self.log(
            "deep.bar",
            "title=\(info.title) crumbs=\(info.breadcrumbs.map(\.name)) actions=\(info.actions.map(\.rawValue))"
        )
        await self.log("deep.page", self.js(session, """
        const header = document.querySelector('body > header');
        return `embedded=${window.__MBR_EMBEDDED__ === true} htmlClass=${document.documentElement.className} headerHeight=${header?.getBoundingClientRect().height} breadcrumbsShown=${getComputedStyle(document.querySelector('nav.breadcrumbs')).display !== 'none'}`;
        """))
        FileHandle.standardError.write(Data("SCREEN deep\n".utf8))
        self.log("done", "deep")
    }

    // MARK: mbr-reload

    /// `.mbr/` edited while the app is in the background (the host script
    /// does it: a `user.css` rule and a `title_prefix` in `config.toml`).
    /// Foregrounding refreshes; the page must reload and show both.
    static func runMbrReload(_ session: VaultSession) async {
        let probe = """
        const h1 = document.querySelector('main h1');
        return `title=${JSON.stringify(document.title)} h1Color=${h1 ? getComputedStyle(h1).color : 'none'}`;
        """
        await self.log("mbr-reload.before", self.js(session, probe))
        self.log("mbr-reload", "waiting; background the app, edit .mbr/user.css and .mbr/config.toml, foreground it")
        FileHandle.standardError.write(Data("SCREEN mbr-reload-ready\n".utf8))
        for _ in 0 ..< 360 where session.lastRefresh?.configReloaded != true {
            try? await Task.sleep(for: .milliseconds(500))
        }
        guard let summary = session.lastRefresh, summary.configReloaded else {
            self.log("mbr-reload.after", "FAIL no refresh reported a config reload")
            return
        }
        // The reload runs right after the summary is recorded.
        try? await Task.sleep(for: .seconds(2))
        let after = await self.js(session, probe)
        let pass = after.contains("RELOADED") && after.contains("rgb(1, 2, 3)")
        self.log(
            "mbr-reload.after",
            "\(pass ? "PASS" : "FAIL") \(after); config=\(summary.configReloaded) "
                + "templates=\(summary.templatesReloaded) assets=\(summary.assetsChanged) "
                + "configError=\(summary.configError ?? "none")"
        )
        self.log("mbr-reload.bar", "title=\(session.web.info.title)")
        self.log("done", "mbr-reload")
    }
}
