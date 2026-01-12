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
- **Edit**: Copy, Paste, Select All
- **View**: Reload (Cmd+R), Toggle DevTools (Cmd+Option+I)
- **History**: Back (Cmd+[), Forward (Cmd+])
- **Window**: Minimize, Zoom, Close (Cmd+W)

**Windows/Linux:**
- **File**: Open Folder, Exit
- **Edit**: Copy, Paste, Select All
- **View**: Reload, Toggle DevTools
- **History**: Back, Forward

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
