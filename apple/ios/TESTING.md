# Device testing checklist

What the simulator cannot show: real file providers, dataless files, iCloud
sync, a phone's speed, and a phone's memory. This supersedes the PR0 spike's
`SPIKE.md`. The simulator results are in [README.md](README.md#simulator-self-test).

**Where results go:** paste the numbers into `design/ios-architecture-research.md`
§12.4 (the "Device, `~/Notes` on iCloud" line and its table). Note anything
that failed under §12.5, or open an issue.

## Setup

1. `./build.sh device`, then install:
   `xcrun devicectl device install app --device <id> ../../target/ios-derived-data/Build/Products/Release-iphoneos/MBR.app`.
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

Settings → **Download files on read**. Off (the default) makes reads of
not-downloaded files fail fast and shows a Downloading page. On makes them wait.
Force-quit and relaunch after changing it (the scan's thread pool reads it at
launch).

| | cold scan | cache-warm ready | pages that failed to render |
|---|---|---|---|
| Off (default) | ____ | ____ | ____ |
| On | ____ | ____ | ____ |

## 4. Remove Download: dataless files and the Downloading page

In Files, long-press Notes → **Remove Download**. Then reopen the vault in MBR.

- [ ] Vault opens; navigation and titles still there (from the cached index): ______
- [ ] `refresh` / `ready` lines report `evicted=` ______
- [ ] Open a note: a **Downloading** page appears, then the note itself once it
      arrives. Time to the note: ______ s
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

## 13. Lifecycle and memory

- [ ] Background the app while a note is open, edit that note on the Mac and let
      it sync, then return. `refresh (foreground): … ~1` and
      `refresh: reloading …` appear, and the page shows the edit: ______
- [ ] `flush: index saved in N ms` on going to the background: ______
- [ ] Leave the app in the background for an hour, then return: the page is
      there, or `page: web content process terminated; reloading …`: ______
- [ ] A large video plays and seeks without the app being killed: ______
