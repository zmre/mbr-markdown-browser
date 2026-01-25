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
| `Ctrl+f` / `Ctrl+b` | Full page down / up |
| `g g` | Go to top of page |
| `G` | Go to bottom of page |
| `H` / `L` | Previous / next sibling page |
| `Ctrl+o` / `Ctrl+i` | History back / forward |

## Panels

| Key | Action |
|-----|--------|
| `/` | Open search |
| `-` or `F2` | Open file browser |
| `Ctrl+g` | Toggle info panel |
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

## macOS GUI Mode

In GUI mode (`mbr -g`), standard macOS shortcuts are available:

| Key | Action |
|-----|--------|
| `Cmd+O` | Open folder |
| `Cmd+R` | Reload page |
| `Cmd+[` | History back |
| `Cmd+]` | History forward |
| `Cmd+Option+I` | Toggle developer tools |
| `Cmd+W` | Close window |
| `Cmd+Q` | Quit application |

Standard Edit menu shortcuts (Cut, Copy, Paste, Undo, Redo, Select All) work as expected in text fields.
