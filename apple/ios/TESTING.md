# Device testing checklist

What the simulator cannot show: real file providers, dataless files, iCloud
sync, a phone's speed, and a phone's memory. This supersedes the PR0 spike's
`SPIKE.md`. The simulator results are in [README.md](README.md#simulator-self-test).

**Where results go:** paste the numbers into `design/ios-architecture-research.md`
§12.4 (the "Device, `~/Notes` on iCloud" line and its table). Note anything
that failed under §12.5, or open an issue.

## Setup

1. Find the phone's UDID with `xcrun devicectl list devices`. Then run
   `MBR_IOS_DEVICE=<udid> ./build.sh device`. Naming the phone lets Xcode
   register it with the team; the first signed build fails with "Your team has
   no devices" until one is registered. Then install and launch:
   `xcrun devicectl device install app --device <udid> ../../target/ios-derived-data/Build/Products/Release-iphoneos/MBR.app`,
   then `xcrun devicectl device process launch --device <udid> com.zmre.mbr.ios`.
   You can also run `./build.sh rust`, open `MBRMobile.xcodeproj` in Xcode, and
   run it on the phone.
2. Logs: in Console.app, select the phone and filter on **subsystem
   `com.zmre.mbr.ios`**. The lines to read:

   | Line | Means |
   |------|-------|
   | `open: MbrApp.open took N ms` | config, templates, router |
   | `ready: fromCache=… scan=N ms ready=N ms …` | indexing done (`scan` = navigation answerable) |
   | `first page: … finished in N ms` | first page on screen |
   | `prewarm: web page ready in N ms` | WebContent spawn, hidden behind the vault list |
   | `GET /path -> 200 … N B in N ms` | one line per request |
   | `refresh (foreground): +a ~c -r evicted=e in N ms` | the stat-diff on returning to the app |
   | `coordinator: write … (replace) in N ms` | a coordinated write |

3. The subtitle under a vault's title shows `open · scan · ready · page` in ms.
   A `c` after `ready` means the index came from the cache. Turn it off in
   Settings → Show timings.

Version tested: ______  Device / iOS: ______  Date: ______

## 1. Pick `~/Notes` on iCloud (~3k notes, 4.8k files)

Vault list → folder-plus → iCloud Drive → Notes → **Open**.

- [ ] Folder could be picked (iOS 26 enables Open only for writable folders): ______
- [ ] Home page renders: ______
- [ ] Any `bookmark stale` line in the log: ______

## 2. Open times

| | open | scan | ready | first page | fromCache |
|---|---|---|---|---|---|
| Cold: first open after install | ____ | ____ | ____ | ____ | false |
| Warm: back to the list, open again | ____ | ____ | ____ | ____ | ____ |
| Cache-warm: force-quit, relaunch, open | ____ | ____ | ____ | ____ | true |
| After Settings → Clear Cache, relaunch | ____ | ____ | ____ | ____ | false |

- [ ] `refresh` diff on a cache-warm open (`ready: … diff=+a/~c/-r`): ______

## 3. The dataless I/O policy

The app always runs with Rust's default: reads of not-downloaded files fail
fast and the page shows Downloading. The **Download files on read** setting
(materialize on read) was removed: on a 2.3 GB iCloud vault, turning it on made
the open download the whole vault, and the scan had not finished after 5+
minutes. `setMaterializeDatalessFiles` is still exported for tests.

| | cold scan | cache-warm ready | pages that failed to render |
|---|---|---|---|
| Off (the only mode) | 718 ms | 545 ms | none |

Measured 2026-10-15 on an iPhone 15 Pro with `~/Notes` (4,452 files).

## 4. Remove Download: dataless files and the Downloading page

In Files, long-press Notes → **Remove Download**. Then reopen the vault in MBR.

- [ ] Vault opens; navigation and titles still there (from the cached index): ______
- [ ] `refresh` / `ready` lines report `evicted=` ______
- [ ] Open a note: a **Downloading** page appears, then the note itself once it
      arrives. Time to the note: ______ s
- [ ] It reads as a normal state, not an error: a moving progress bar,
      "Downloading from iCloud…" and the note's name, no "503"; the bar
      title is the note's name: ______
- [ ] Log shows `coordinator: iCloud download started for …`: ______
- [ ] WebKit re-requests the page by itself every ~2 s (repeated
      `GET … -> 503 … [Retry-After 2s, Refresh 2]` lines). It did on the
      simulator; if it doesn't on the device, look for
      `downloading: … retrying from Swift`: ______
- [ ] Content search with files evicted: results come back (evicted files are
      skipped and counted in the response's `not_downloaded`), with no stall: ______
- [ ] Afterwards: long-press Notes → **Keep Downloaded**, and note that the
      onboarding's tip is right: ______

## 5. Search

Open search (magnifier, or `/` with a keyboard). The server time is in the log:
`POST /.mbr/search -> 200 … in N ms`.

| Query | Scope | Results | Server ms | Feels instant? |
|-------|-------|---------|-----------|----------------|
| a note title (metadata only) | All | ____ | ____ | ____ |
| a word in many notes | All (content) | ____ | ____ | ____ |
| the same, second time | All | ____ | ____ | ____ |

- [ ] Search modal with the on-screen keyboard up: input and results usable: ______

## 6. Task toggle round trip, and a conflict

Open a note with a `- [ ]` task and tap its checkbox.

- [ ] `POST /.mbr/task -> 200 … in ___ ms` and `coordinator: write … (replace) in ___ ms`
- [ ] On the Mac, the line shows `- [x] … @done(…)` after iCloud syncs: ______ s
- [ ] **409:** without reloading the phone's page, edit that same line on the
      Mac and let it sync. Then toggle it on the phone. Expect `POST /.mbr/task -> 409`
      and an error alert, not a clobber; Refresh then shows the Mac's version: ______
- [ ] Coordinated write fallback: any `atomic replace … failed; writing in place`
      line (expected only on SMB): ______

## 7. Flashcard review write

Open a `type: flashcard` note → Review flashcards → reveal → rate.

- [ ] `POST /.mbr/flashcard-review -> 200`, plus a coordinated write: ______ ms
- [ ] The Mac's copy gains `- YYYY-MM-DD HH:MM - Good` under `___Review History___`: ______
- [ ] A second rating in the same session, with no 409: ______

## 8. Bookmark persistence

- [ ] Force-quit, relaunch: Notes reopens without re-picking: ______
- [ ] Reboot the phone, relaunch: Notes reopens: ______
- [ ] If a vault ever says it cannot be opened: **Re-pick Folder…** restores
      it, keeping its name and cache: ______

## 9. Other providers

| Provider | Pickable? | Renders? | Toggle writes (and syncs back)? | Outside edit appears after Refresh? |
|----------|-----------|----------|----------------------------------|-------------------------------------|
| Working Copy repo | ____ | ____ | ____ | ____ |
| Dropbox folder | ____ | ____ | ____ | ____ |
| On My iPhone folder | ____ | ____ | ____ | ____ |
| Google Drive (expect: not pickable) | ____ | n/a | n/a | n/a |

## 10. Single file opened in place, then the folder

In Files, open a `.md` from a cloud folder that is *not* a vault: long-press →
Share → **MBR**, or "Open with".

- [ ] It opens in place, as one page; the log has `document: open …` and
      `MbrApp.openFile`: ______
- [ ] A task in it toggles and the change reaches the original file: ______
- [ ] Tap **Grant Folder Access** (folder-plus in the toolbar). The picker
      starts in the file's folder. Open it: the folder becomes a vault, opened at
      that same note, and its links and images now resolve: ______
- [ ] A Google Drive / OneDrive file opens this way too (single file only): ______
- [ ] With a vault open (a note on screen), Share → MBR another `.md`: the
      file replaces the vault (`close: <vault>`, `MbrApp.openFile`), and Back
      returns to the vault list: ______
- [ ] Grant Folder Access on a Google Drive / Sync.com file: Open stays dimmed in
      its folder; Cancel keeps the file open and shows "No folder opened": ______

## 11. iPad

- [ ] Full screen: the desktop layout (sidebar visible): ______
- [ ] Split View at ½ and ⅓ width, and Slide Over: which widths get the
      desktop layout, which the phone layout, and does anything break?: ______
- [ ] Stage Manager: resize the window across 1024 px: ______
- [ ] A hardware keyboard: `/`, `Cmd+K`, `p` (flashcards), `t` (tasks): ______

## 12. Offline (airplane mode)

With Notes kept downloaded, turn on airplane mode and force-quit.

- [ ] Vault opens and pages render: ______
- [ ] Search works: ______
- [ ] A task toggle writes locally and syncs once back online: ______
- [ ] A not-downloaded note shows the Downloading page; does it recover once
      back online?: ______
- [ ] That page says **"You're offline."** and "This note will download when
      you're back online." under the note's name, with no progress bar; the
      log shows `network: offline`. Turn airplane mode off with the page
      open: it switches back to "Downloading from iCloud…" (`network:
      online`) and the note loads by itself: ______

## 13. Lifecycle and memory

- [ ] **`.mbr/` on refresh:** with a note open, on the Mac edit the vault's
      `.mbr/user.css` (e.g. `main h1 { color: red }`) and `.mbr/config.toml`
      (e.g. `title_prefix`), let it sync, then bring MBR back (or ⋯ →
      Refresh). The log shows `.mbr: config=true … assets=true` and
      `Reopened with the changed configuration in N ms (index carried over…)`;
      the page shows both changes: ______
- [ ] Break `config.toml` on the Mac: the "Configuration ignored" notice
      appears after a refresh; fix it and refresh again: the notice goes: ______

- [ ] Background the app while a note is open, edit that note on the Mac and let
      it sync, then return. `refresh (foreground): … ~1` and
      `refresh: reloading …` appear, and the page shows the edit: ______
- [ ] `flush: index saved in N ms` on going to the background: ______
- [ ] Leave the app in the background for an hour, then return: the page is
      there, or `page: web content process terminated; reloading …`: ______
- [ ] A large video plays and seeks without the app being killed: ______

## 14. The native bar (header hidden)

On a note a few folders deep with a long title:

- [ ] mbr's own web header and breadcrumbs are gone; the iOS bar shows the
      page title (ellipsized) and the timings under it: ______
- [ ] Tap the title: the breadcrumb path, nearest folder first, then Home.
      Each one opens: ______
- [ ] ⋯ lists only what works on that page: Browse, Search, Edit, Info, Tasks;
      Flashcards on a `type: flashcard` note; Slides on a `style: slides` note.
      Each opens its panel: ______
- [ ] ⋯ → Refresh, Home, Settings and Open Other Vault… all work: ______
- [ ] Back goes back through the vault's pages; on the first page it returns
      to the vault list. A long press lists recent pages: ______
- [ ] Swipe from the left edge goes back a page: ______
- [ ] Nothing sits under the notch, the navigation bar or the home indicator
      (portrait and landscape; the deck, the search modal, the tasks overlay,
      the info panel): ______

## 15. Narrow screens (the app, and phone Safari over Tailscale)

- [ ] In phone Safari against `mbr -s`, the header stays on one line and the
      title is readable; breadcrumbs are one line ("… › parent › page") and
      "…" expands the full path: ______
- [ ] The editor (⋯ → Edit) fills the screen with readable margins; selecting
      text shows the formatting toolbar wrapped within the screen, with no
      sideways scrolling: ______
- [ ] The tasks overlay (⋯ → Tasks) is one column; the folder button opens the
      folder list and picking one closes it; the filter field, ⚙ and ✕ are
      easy to hit: ______
- [ ] The search modal with the keyboard up: results stay visible above it: ______
- [ ] No keyboard-shortcut footers on the phone (tasks overlay, search
      modal, flashcard deck); Browse Media still shows in search. With an iPad
      keyboard/trackpad attached they may reappear: ______

## 16. Home screen

- [ ] MBR's icon (the tree) appears on the home screen and in the app
      switcher: ______
