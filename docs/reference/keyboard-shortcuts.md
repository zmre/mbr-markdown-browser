---
title: Keyboard Shortcuts
description: Quick reference for all keyboard shortcuts
order: 4
---

# Keyboard Shortcuts

mbr provides vim-style keyboard shortcuts for efficient navigation. Press `?` at any time to view an in-app help overlay.

## Page Navigation

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll down / up (one line) |
| `Ctrl+d` / `Ctrl+u` | Half page down / up |
| `Ctrl+f` / `Ctrl+b` | Full page down / up (`Ctrl+f` is macOS-only — see note below) |
| `g g` | Go to top of page |
| `G` | Go to bottom of page |
| `H` / `L` | Previous / next sibling page |
| `Ctrl+o` / `Ctrl+i` | History back / forward |

**Note on `Ctrl+f`:** full-page-down is bound on macOS only, where `Cmd+F` is the find
key. On Windows and Linux `Ctrl+F` is left alone so it reaches find-in-page — the
browser's own in server and static modes, mbr's in GUI mode. Use `Ctrl+b`, `Space` or
`PageDown` to page forward there.

## Panels

| Key | Action |
|-----|--------|
| `/` | Open search |
| `-` or `F2` | Open file browser |
| `=` | Open media browser |
| `Ctrl+g` | Toggle info panel |
| `t` | Open the task browser (server and GUI modes only) |
| `e` | Open the editor for the current file (when [editing](../modes/editing/) is enabled) |
| `Esc` | Close current panel |

## Quick Navigation (Fuzzy Nav)

| Key | Action |
|-----|--------|
| `f` | Open links out (outbound links from current page) |
| `F` | Open links in (backlinks to current page) |
| `T` | Open table of contents (headings) |

## Search Panel (when open)

| Key | Action |
|-----|--------|
| `Ctrl+n` / `Ctrl+p` | Navigate results down / up |
| `↑` / `↓` | Navigate results |
| `Enter` | Open selected result |
| `Ctrl+d` / `Ctrl+u` | Scroll results half page |
| `Esc` | Close search |

## File Browser (when open)

| Key | Action |
|-----|--------|
| `j` / `k` / `↑` / `↓` | Navigate tree |
| `Ctrl+n` / `Ctrl+p` | Navigate tree |
| `h` | Collapse folder / go to parent |
| `l` or `Enter` | Expand folder / open file |
| `o` | Open in new tab |
| `Ctrl+d` / `Ctrl+u` | Scroll panel half page |
| `Esc` | Close browser |

## Task Browser (when open)

The task browser (`t`) is a live view of the tasks in your markdown, so it exists in
server and GUI modes only — a static build has no files to re-read. Disable it with
`--no-tasks`. See [Task Browser](../markdown/tasks/) for the full guide.

| Key | Action |
|-----|--------|
| `Ctrl+n` / `Ctrl+p` | Navigate tasks and headings |
| `↑` / `↓` | Navigate tasks and headings |
| `←` / `→` | Collapse / expand the focused group |
| `Enter` | Open the focused task in its file, or toggle the focused heading |
| `Space` | Complete / reopen the focused task (when [editing](../modes/editing/) is enabled) |
| `x` | Cancel / reopen the focused task (when editing is enabled) |
| `Tab` | Switch between the folder pane and the results pane |
| `Ctrl+d` / `Ctrl+u` | Scroll the active pane half a page |
| `Ctrl+f` / `Ctrl+b` | Scroll the active pane a full page |
| `Esc` | Close the filter options, then the panel |

The filter field keeps focus the whole time, so `Space`, `x`, `←` and `→` reach it
until you move onto a task with the arrow keys — and typing in the field hands them
back. Filtering for "buy milk" therefore works as you would expect.

## Task Checkboxes (in a page)

With [editing](../modes/editing/) enabled, the checkboxes in a rendered page are
clickable.

| Action | Result |
|--------|--------|
| Left click | Complete / reopen the task |
| Right click | Cancel / reopen the task |

## Fuzzy Nav Modal (when open)

| Key | Action |
|-----|--------|
| `Tab` | Switch between tabs (Links Out / Links In / ToC) |
| `Shift+Tab` | Switch tabs in reverse |
| `Ctrl+n` / `Ctrl+p` | Navigate results |
| `↑` / `↓` | Navigate results |
| `Enter` | Open selected item |
| `Esc` | Close modal |

## Help

| Key | Action |
|-----|--------|
| `?` | Toggle keyboard shortcuts overlay |

## Find in Page (GUI mode)

GUI mode (`mbr -g`) has no browser chrome, so it ships its own find bar, driven from the
native **Edit** menu. In server and static modes this bar is not present at all and your
browser's built-in find works as usual.

| Action | macOS | Windows/Linux |
|--------|-------|---------------|
| Open find bar | `Cmd+F` | `Ctrl+F` |
| Find next | `Cmd+G` | `F3` |
| Find previous | `Shift+Cmd+G` | `Shift+F3` |
| Find next / previous (bar focused) | `Enter` / `Shift+Enter` | `Enter` / `Shift+Enter` |
| Close find bar | `Esc` | `Esc` |

`F3` is used off macOS because `Ctrl+G` is already the info panel toggle. On macOS the
info panel keeps `Ctrl+G`, and `Cmd+G` belongs to find-next as the platform expects.
Reloading the page (including live reload) closes the bar and clears its highlights.

## macOS GUI Mode

In GUI mode (`mbr -g`), standard macOS shortcuts are available:

| Key | Action |
|-----|--------|
| `Cmd+O` | Open folder |
| `Cmd+R` | Reload page |
| `Cmd+[` | History back |
| `Cmd+]` | History forward |
| `Cmd+F` | Find in page |
| `Cmd+G` / `Shift+Cmd+G` | Find next / previous |
| `Cmd+Option+I` | Toggle developer tools |
| `Cmd+W` | Close window |
| `Cmd+Q` | Quit application |

Standard Edit menu shortcuts (Cut, Copy, Paste, Undo, Redo, Select All) work as expected in text fields.
