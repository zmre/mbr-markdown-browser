---
title: GUI Mode
description: Native browser window with mbr -g
order: 1
---

# GUI Mode

GUI mode launches a native desktop window with an embedded web view, providing a dedicated markdown browsing experience.

## Launching

```bash
mbr -g /path/to/notes
```

If no path is provided, mbr uses the current directory.

## Features

### Native Menu Bar

mbr provides platform-appropriate menus:

**macOS:**
- **mbr** menu: About, Preferences, Quit (Cmd+Q)
- **File**: Open Folder (Cmd+O)
- **Edit**: Copy, Paste, Select All, Find… (Cmd+F), Find Next (Cmd+G), Find Previous (Shift+Cmd+G)
- **View**: Reload (Cmd+R), Toggle DevTools (Cmd+Option+I)
- **History**: Back (Cmd+[), Forward (Cmd+])
- **Window**: Minimize, Zoom, Close (Cmd+W)

**Windows/Linux:**
- **File**: Open Folder, Exit
- **Edit**: Copy, Paste, Select All, Find… (Ctrl+F), Find Next (F3), Find Previous (Shift+F3)
- **View**: Reload, Toggle DevTools
- **History**: Back, Forward

### Find in Page

GUI mode has no browser chrome, so Cmd+F/Ctrl+F has nothing to open — the webview
underneath (WKWebView, WebView2 or WebKitGTK) ships no find bar of its own, and wry
exposes no find API. mbr therefore provides its own, opened from the **Edit** menu.

It highlights every match, shows a "3 of 17" counter, and supports case-sensitive
matching. `Enter`/`Shift+Enter` step through matches while the bar is focused, and `Esc`
closes it. Multi-word queries match across soft-wrapped lines but never across a
paragraph or other block boundary.

This bar exists **only** in GUI mode. Server mode (`mbr -s`) and static builds never
include it, so your browser's native find is left untouched.

### Keyboard Shortcuts

**Window Controls:**

| Action | macOS | Windows/Linux |
|--------|-------|---------------|
| Open Folder | Cmd+O | Ctrl+O |
| Reload Page | Cmd+R | Ctrl+R |
| Go Back | Cmd+[ | Alt+Left |
| Go Forward | Cmd+] | Alt+Right |
| Close Window | Cmd+W | Ctrl+W |
| Quit | Cmd+Q | Alt+F4 |
| Developer Tools | Cmd+Option+I | Ctrl+Shift+I |

**In-Page Navigation:**

| Key | Action |
|-----|--------|
| `-` | Open file browser sidebar |
| `/` | Open search dialog |
| `Escape` | Close sidebar or search |
| `↑` / `↓` | Navigate items |
| `Enter` | Open selected |

See [Server Mode: File Browser Sidebar](server.md#file-browser-sidebar) for full details on the browser panel.

### Live Reload

When files change on disk, the GUI automatically reloads the current page. This enables a smooth writing workflow:

1. Open your notes in mbr GUI
2. Edit files in your preferred editor
3. See changes instantly in mbr

### Switching Directories

Use **File → Open Folder** (Cmd+O) to switch to a different markdown repository without restarting mbr.

### External Links

The mbr window is a viewer for your notes, not a web browser, so it only ever
displays pages from mbr's own local server. Anything else is handed to the
system, exactly as if you had clicked it in another native app:

| Link | What happens |
|------|--------------|
| `docs/guide.md`, `/notes/`, `#section` | Navigates inside the mbr window |
| `https://example.com` | Opens in your **default browser** |
| `mailto:`, `tel:`, `message:`, `zoommtg:`, `x-devonthink-item:`, … | Opens in the application registered for that scheme |
| `javascript:`, `vbscript:`, `data:` | Refused — neither followed nor handed to the system |

Application schemes are recognised by *shape*, not from a list, so a scheme mbr
has never heard of works as long as something on your machine claims it. The URL
reaches the system byte for byte: a `message:` link addresses a message by its
`Message-ID`, whose angle brackets arrive percent-encoded as `%3C`/`%3E`, and
re-encoding or decoding them would hand your mail client an ID it cannot find.

`target="_blank"` links and `window.open()` follow the same rule: same-origin
popups open a linked mbr window (this is what Reveal.js speaker notes need),
while external ones go to your browser rather than opening a second mbr window
around somebody else's site.

**Embedded content is unaffected.** A YouTube embed, or any other `<iframe>`,
keeps loading in place — embeds are not links and are never handed to the
system. This is not incidental: the webview's navigation callback cannot tell an
`<iframe>` load from a click, so mbr splits the work. Application schemes are
decided there (no frame can ever load a `mailto:`), while ordinary `http`/`https`
links are recognised in the page itself, where a click is unmistakably a click.
`src/external_open.rs` carries the full reasoning.

The hand-off is built in. mbr does not shell out to `open`, `xdg-open` or
`rundll32`; it calls `NSWorkspace` on macOS, `ShellExecuteW` on Windows and gio's
default-application launcher on Linux.

#### Launching applications is GUI-only, by design

**Only the GUI window ever hands a URL to the operating system.** Server mode
(`mbr -s`) and static builds deliberately do nothing with external links: they
delegate to the browser you are visiting with, which already opens `https://`
itself and hands `mailto:`, `message:` and friends to your own machine's
registered applications.

That is a security boundary, not a missing feature. Launching an application is
something an interactive window does on behalf of the person sitting in front of
it. A process answering HTTP has no such person, and **a server must never be
induced to launch applications on its host** — a shared or remote `mbr -s` would
otherwise let anyone who can reach the page start programs on the machine
running it.

mbr enforces this in three independent places, so no single mistake re-opens it:

| Layer | What it does |
|-------|--------------|
| Runtime guard | The launcher refuses unless a GUI window is actually running in the process, and refuses *before* touching the OS. Because the GUI is compiled in by default, a server-mode binary still contains the launcher; this is what keeps it inert regardless of who calls it. Refusals are logged at `warn`. |
| Templates | The in-page click handler is emitted only on GUI pages. Server and static pages never load it. |
| The component itself | Checks the mode again at runtime and does nothing outside GUI mode — because `.mbr/` template overrides mean your repository's own templates, not mbr's, may decide what gets mounted. |

If you see `Refusing to open … : handing URLs to the operating system is
GUI-only` in a server log, nothing was launched; something asked a windowless
process to start an application and was turned down.

## macOS App Bundle

The macOS release includes `MBR.app`, a proper application bundle that:

- Appears in Spotlight search
- Has a dock icon
- Integrates with the system menu bar
- Includes the QuickLook extension

### Installing the App Bundle

1. Download the macOS release from GitHub
2. Move `MBR.app` to `/Applications`
3. Double-click to launch, or run from Terminal:

```bash
open -a MBR /path/to/notes
```

### Command-Line Access

Even with the app bundle installed, you can use the command-line interface:

```bash
# Use the binary inside the app bundle
/Applications/MBR.app/Contents/MacOS/mbr -s ~/notes
```

Or add an alias to your shell:

```bash
alias mbr="/Applications/MBR.app/Contents/MacOS/mbr"
```

## Developer Tools

Press **Cmd+Option+I** (macOS) or **Ctrl+Shift+I** (Windows/Linux) to open developer tools. This is useful for:

- Debugging custom CSS
- Inspecting component behavior
- Viewing network requests
- Checking console errors

## Window Behavior

- **Persistence**: Window size and position are remembered between sessions
- **Focus**: Uses the system's native window management
- **Multi-window**: Each mbr instance opens its own window

## Troubleshooting

### Window Won't Open

If the GUI window fails to open:

1. Check if a server is already running on port 5200
2. Try with verbose logging: `mbr -g -vv ~/notes`
3. Ensure your system allows the app (macOS Gatekeeper)

### Slow Performance

If rendering is slow:

1. Check file count in the repository
2. Ensure fast storage (SSD recommended for large repos)
3. Consider using server mode for very large repositories

### macOS Gatekeeper

If macOS blocks the app:

```bash
# Remove quarantine flag
xattr -dr com.apple.quarantine /Applications/MBR.app
```

Or right-click the app and select "Open" to bypass Gatekeeper once.
