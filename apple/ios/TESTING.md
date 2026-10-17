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

Version tested: branch `2026-10-15-editor-plugins` (Release, through 37b29ed)
Device / iOS: iPhone 15 Pro, iOS 27.0.1; iPad Pro 12.9" 3rd gen (iPad8,7)
Date: 2026-10-15 and 2026-10-16

Vault: `~/Notes` on iCloud, 4,452–4,454 files (~3,150 notes), ~2.3 GB.
Results are for the iPhone unless a line says otherwise.

## 1. Pick `~/Notes` on iCloud (~3k notes, 4.8k files)

Vault list → folder-plus → iCloud Drive → Notes → **Open**.

- [x] Folder could be picked (iOS 26 enables Open only for writable folders): yes
- [x] Home page renders: yes
- [x] Any `bookmark stale` line in the log: none

## 2. Open times

All times in ms, 4,452 files.

|                                                    | open | scan | ready | first page | fromCache |
|----------------------------------------------------|------|------|-------|------------|-----------|
| Cold: first open after installing a new build ¹    | 15   | 518  | 713   | 163        | false     |
| Warm: back to the list, open again                 | 23   | 33   | 466   | 124        | true      |
| Cache-warm: force-quit, relaunch, open             | 25   | 43   | 545   | 278        | true      |
| After Settings → Clear Cache, relaunch             | 34   | 718  | 821   | 238        | false     |

¹ A new build changes the mbr version in the snapshot fingerprint, so its
cache is ignored and the vault is scanned: the same as a first open after
install, with the folder listings already on the device. A device that has
never listed the vault is much slower; see §11.

- [x] `refresh` diff on a cache-warm open (`ready: … diff=+a/~c/-r`): `+0/~0/-0`
      on both the warm and the cache-warm open

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

Run on a subfolder of ~12 notes (Remove Download on the whole vault was not
needed: iOS had already offloaded ~500 files of it by itself).

- [x] Vault opens; navigation and titles still there (from the cached index): yes
- [x] `refresh` / `ready` lines report `evicted=`: `refresh (foreground)`
      reported `evicted=12` in 308 ms
- [x] Open a note: a **Downloading** page appears, then the note itself once it
      arrives. Time to the note: ~2 s
- [x] It reads as a normal state, not an error: a moving progress bar,
      "Downloading from iCloud…" and the note's name, no "503"; the bar
      title is the note's name: yes
- [x] Log shows `coordinator: iCloud download started for …`: yes, after the
      download request
- [x] WebKit re-requests the page by itself every ~2 s (repeated
      `GET … -> 503 … [Retry-After 2s, Refresh 2]` lines). It did on the
      simulator; if it doesn't on the device, look for
      `downloading: … retrying from Swift`: WebKit honoured `Refresh` on the
      device too
- [x] Content search with files evicted: results come back (evicted files are
      skipped and counted in the response's `not_downloaded`), with no stall:
      265–347 ms, `not_downloaded` 514
- [x] Afterwards: long-press Notes → **Keep Downloaded**, and note that the
      onboarding's tip is right: the tip is accurate. The next refresh saw ~15
      changed files; search then reported 503 not downloaded (still arriving)

**Bug found on the way:** a large iCloud folder whose *listing* had never been
downloaded showed as empty (131 folders, 705 files). Fixed in ec83873
(materialize directory listings, reload a page that was rendered from a stale
index; also removed the Download files on read setting, §3) and 9a9bc9b
(request folders whose listing is not downloaded; the refresh indexes them
when they arrive). That folder's first open then took ~19 s of listing in the
background.

## 5. Search

Open search (magnifier, or `/` with a keyboard). The server time is in the log:
`POST /.mbr/search -> 200 … in N ms`.

| Query                        | Scope         | Results | Server ms | Feels instant? |
|------------------------------|---------------|---------|-----------|----------------|
| a note title (metadata only) | All           | 41      | 462       | yes            |
| a word in many notes         | All (content) | 988     | 242       | yes            |
| another word in many notes   | All (content) | 486     | 231       | yes            |
| the same, second time        | All           | 486     | 231       | yes            |

The vault had 503 files not downloaded at the time; each search skipped and
counted them.

- [x] Search modal with the on-screen keyboard up: input and results usable: yes
      (the native search sheet). The web modal in phone Safari: see §15

## 6. Task toggle round trip, and a conflict

Open a note with a `- [ ]` task and tap its checkbox.

- [x] `POST /.mbr/task -> 200` and `coordinator: write … (replace)`: yes
      (same range as §7: request tens of ms, coordinated write 12–23 ms)
- [x] On the Mac, the line shows `- [x] … @done(…)` after iCloud syncs: < 10 s
- [ ] **409:** without reloading the phone's page, edit that same line on the
      Mac and let it sync. Then toggle it on the phone. Expect `POST /.mbr/task -> 409`
      and an error alert, not a clobber; Refresh then shows the Mac's version:
      **not yet retested.** The first try hit a different case: the Mac's new
      version had not downloaded yet, so `/.mbr/raw` answered 404 and the
      page said "Could not read the file to update it." Nothing was written
      and the Mac's edit was safe. Fixed in 7fdaf94 (a not-downloaded note
      answers 503, starts the download and says what happened). 9e34e40 also
      makes writes check the hash of the version the page shows, so a toggle
      can no longer land on a different task that reads the same
- [x] Coordinated write fallback: any `atomic replace … failed; writing in place`
      line (expected only on SMB): none

**Crash found:** the first toggle attempt crashed the app. `Log.info` wrote to
stderr after the `devicectl` console had dropped (the phone slept), and the
`EPIPE` aborted. Fixed in 8cf83ae; verified later when the console dropped
again and the app kept running.

## 7. Flashcard review write

Open a `type: flashcard` note → Review flashcards → reveal → rate.

- [x] `POST /.mbr/flashcard-review -> 200`, plus a coordinated write: request
      26–42 ms, coordinated write 12–23 ms
- [x] The Mac's copy gains `- YYYY-MM-DD HH:MM - Good` under `___Review History___`:
      yes, history lines correct
- [x] A second rating in the same session, with no 409: yes, three ratings

A rating button kept its hover look after a tap (sticky hover on touch);
fixed in 71c912a.

## 8. Bookmark persistence

- [x] Force-quit, relaunch: Notes reopens without re-picking: yes. After the
      user swipes the app away, the app shows the vault list rather than the
      last page (iOS drops state restoration then); the vault opens from it
      without a re-pick
- [ ] Reboot the phone, relaunch: Notes reopens: skipped
- [ ] If a vault ever says it cannot be opened: **Re-pick Folder…** restores
      it, keeping its name and cache: not seen (no vault failed to open)

## 9. Other providers

| Provider                            | Pickable?  | Renders? | Toggle writes (and syncs back)? | Outside edit appears after Refresh? |
|-------------------------------------|------------|----------|---------------------------------|-------------------------------------|
| Working Copy repo                   | not tested | —        | —                               | —                                   |
| Dropbox folder                      | yes        | yes      | yes                             | yes, after b0d0d2a ¹                |
| On My iPhone folder                 | yes        | yes      | yes (seen in Files)             | yes                                 |
| Google Drive (expect: not pickable) | no ²       | n/a      | n/a                             | n/a                                 |

¹ At first a Mac edit was not seen by Refresh until the folder had been
browsed in Files: the provider's local replica was stale. Fixed in b0d0d2a
(ask the file provider for updates before a Refresh); retest passed with
`provider: refreshed in 1984 ms` and the page's file changed (`~1`). The
2 s wait is paid even when nothing changed (open in TODO.md).

² The picker's Open button is enabled but does nothing, with no feedback.

## 10. Single file opened in place, then the folder

In Files, open a `.md` from a cloud folder that is *not* a vault: long-press →
Share → **MBR**, or "Open with".

- [x] It opens in place, as one page; the log has `document: open …` and
      `MbrApp.openFile`: yes (Open With on a Sync.com file, from the vault list)
- [x] A task in it toggles and the change reaches the original file: yes,
      saved back through the provider
- [ ] Tap **Grant Folder Access** (folder-plus in the toolbar). A banner under
      the bar names the folder ("Pick the folder: Dropbox › Projects › notes")
      for a moment, then the picker opens. Open that folder: it becomes a vault,
      opened at that same note, and its links and images now resolve: the banner
      shows the folder's path (2673b59); picking it works. The provider name
      was not shown (iOS did not report one)
      - The picker does **not** start in the file's folder for a third-party
        File Provider location: iOS 27.0.1 ignores `directoryURL` there
        (Dropbox, iPhone 15 Pro, 2026-10-16: it opened at the last folder
        picked, with the file's scope started and the parent from
        `.parentDirectoryURLKey`). The banner is how you find the folder.
      - Log: `picker: start … (parent from resource key, file scope
        started=true, provider=…, label "…")`, `picker: provider name
        localizedName=…, displayName=…, containerDisplayName=…` (whether iOS
        names the provider at all) and `picker: opening in …`: the right
        directory was passed; iOS ignored it even with the file scope started,
        the parent from the resource key and a UIKit presentation
      - The picker follows dark mode (dark sheet, readable title): it was a
        white sheet with an unreadable title; fixed in 37b29ed, not yet
        rechecked on the device
- [ ] A Google Drive / OneDrive file opens this way too (single file only): not tested
- [x] With a vault open (a note on screen), Share → MBR another `.md`: the
      file replaces the vault (`close: <vault>`, `MbrApp.openFile`), and Back
      returns to the vault list: at first the log showed `document: open` and
      nothing happened; fixed in fd5116c, recheck passed
- [x] Grant Folder Access on a Google Drive / Sync.com file: Open stays dimmed in
      its folder; Cancel keeps the file open and shows "No folder opened", with
      the folder's name and **Try Again**: a Sync.com folder cannot be picked
      (provider limitation); the hint that says so came in 2673b59/37b29ed

## 11. iPad

iPad Pro 12.9" 3rd gen (iPad8,7). Prewarm: 1299 ms. Sample vault: open 57,
scan 58, ready 69, first page 699 ms. `~/Notes`, first open ever on this
device: open 5877 ms (four `.mbr/` files materializing on the Swift thread,
`mbrDownloads=4`), first page 2022 ms, scan 16494 ms, ready 16630 ms (every
folder listing fetched from iCloud for the first time; the page shows at once
from what is local).

- [x] Full screen: the desktop layout (sidebar visible): yes for the sample
      vault. `~/Notes` shows no docked sidebar, which is that vault's own
      browse style, not a bug
- [ ] Split View at ½ and ⅓ width, and Slide Over: which widths get the
      desktop layout, which the phone layout, and does anything break?: not
      available on this iPad (Stage Manager instead)
- [x] Stage Manager: resize the window across 1024 px: the layout reflows
- [x] A hardware keyboard: `/`, `Cmd+K`, `p` (flashcards), `t` (tasks): `/`
      and `Cmd+K` work; `p` and `t` not checked

## 12. Offline (airplane mode)

With Notes kept downloaded, turn on airplane mode and force-quit.

- [x] Vault opens and pages render: yes
- [x] Search works: yes
- [x] A task toggle writes locally and syncs once back online: yes; an edit
      and a new task too, all synced to the Mac after airplane mode went off
- [x] A not-downloaded note shows the Downloading page; does it recover once
      back online?: yes, by itself. While offline the page spun with no
      hint that the device was offline; fixed by the check below (dfb318a)
- [ ] That page says **"You're offline."** and "This note will download when
      you're back online." under the note's name, with no progress bar; the
      log shows `network: offline`. Turn airplane mode off with the page
      open: it switches back to "Downloading from iCloud…" (`network:
      online`) and the note loads by itself: pass (dfb318a)

## 13. Lifecycle and memory

- [ ] **`.mbr/` on refresh:** with a note open, on the Mac edit the vault's
      `.mbr/user.css` (e.g. `main h1 { color: red }`) and `.mbr/config.toml`
      (e.g. `title_prefix`), let it sync, then bring MBR back (or ⋯ →
      Refresh). The log shows `.mbr: config=true … assets=true` and
      `Reopened with the changed configuration in N ms (index carried over…)`;
      the page shows both changes: **failed at first**: the edited
      `config.toml` was dataless when the refresh read it (`EDEADLK`), so
      the app showed "Configuration ignored" and used defaults, and the open
      page was not reloaded for `user.css` (later pages had it). Fixed in
      82c5962 (a `.mbr/` file edited elsewhere is requested and used once it
      downloads). Redo: `theme.css`/`user.css` changes applied to the open
      note, no "Configuration ignored": pass
- [ ] Break `config.toml` on the Mac: the "Configuration ignored" notice
      appears after a refresh; fix it and refresh again: the notice goes:
      not run deliberately (seen only through the bug above)

- [x] Background the app while a note is open, edit that note on the Mac and let
      it sync, then return. `refresh (foreground): … ~1` and
      `refresh: reloading …` appear, and the page shows the edit: pass
- [ ] `flush: index saved in N ms` on going to the background: not recorded
      (the console had dropped)
- [ ] Leave the app in the background for an hour, then return: the page is
      there, or `page: web content process terminated; reloading …`: skipped
- [x] A large video plays and seeks without the app being killed: 452 MB and
      1.5 GB videos both play and scrub

## 14. The native bar (header hidden)

On a note a few folders deep with a long title. Every check passed. So did
the follow-ups from TODO.md: the slides deck in landscape hides the bar and
the status bar and shows its ✕; the editor hides the bar, turns the edge
swipes off, and Cancel brings both back; the Info panel's ✕ sits below the
bar. Gap: the app has no way to start a review note (no text-selection or ⋯
entry); open in TODO.md.

- [x] mbr's own web header and breadcrumbs are gone; the iOS bar shows the
      page title (ellipsized) and the timings under it: pass
- [x] Tap the title: the breadcrumb path, nearest folder first, then Home.
      Each one opens: pass
- [x] ⋯ lists only what works on that page: Browse, Search, Edit, Info, Tasks;
      Flashcards on a `type: flashcard` note; Slides on a `style: slides` note.
      Each opens its panel: pass
- [x] ⋯ → Refresh, Home, Settings and Open Other Vault… all work: pass
- [x] Back goes back through the vault's pages; on the first page it returns
      to the vault list. A long press lists recent pages: pass
- [x] Swipe from the left edge goes back a page: pass. (Since then a
      left-edge swipe opens Browse and WebKit's back swipe is off; Back is
      the bar's button.)
- [x] Nothing sits under the notch, the navigation bar or the home indicator
      (portrait and landscape; the deck, the search modal, the tasks overlay,
      the info panel): pass

## 15. Narrow screens (the app, and phone Safari over Tailscale)

Setting up the server turned up two CLI issues, fixed in a9b7e28: an error at
startup (here `--edit` on `0.0.0.0` without a token, which is refused by
design) printed as a Rust `Debug` dump, and `cargo run` needed `--bin mbr`.

- [x] In phone Safari against `mbr -s`, the header stays on one line and the
      title is readable; breadcrumbs are one line ("… › parent › page") and
      "…" expands the full path: pass
- [ ] The editor (⋯ → Edit) fills the screen with readable margins; selecting
      text shows the formatting toolbar wrapped within the screen, with no
      sideways scrolling: not recorded
- [x] The tasks overlay (⋯ → Tasks) is one column; the folder button opens the
      folder list and picking one closes it; the filter field, ⚙ and ✕ are
      easy to hit: pass
- [x] The search modal with the keyboard up: results stay visible above it:
      at first the results ran under the keyboard until it was dismissed;
      fixed in e2ba412, Safari recheck passed. The QuickType suggestion bar
      over the keyboard is gone with 602aa13 (no autocorrect or predictions
      on the field)
- [ ] No keyboard-shortcut footers on the phone (tasks overlay, search
      modal, flashcard deck); Browse Media still shows in search. With an iPad
      keyboard/trackpad attached they may reappear: in phone Safari the
      search modal still showed keyboard hints and Safari's form accessory
      bar (whether this happens in the app too was not checked)

## 16. Home screen

- [x] MBR's icon (the tree) appears on the home screen and in the app
      switcher: pass
