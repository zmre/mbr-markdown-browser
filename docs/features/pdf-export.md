---
title: PDF Export
description: Save a note as a paginated PDF, exactly as mbr renders it — from the GUI's File menu or with mbr --pdf in a script
order: 17
---

# PDF Export

**Works in:** GUI · CLI (`mbr --pdf`) — on macOS, Windows and Linux desktops. Not in server mode, static builds, QuickLook or the iOS app.

mbr can save a note as a PDF that looks like the page on screen: mermaid
diagrams drawn, math typeset, code highlighted, images loaded, tables and
contact cards laid out. The text is real text (selectable and searchable, fonts
embedded) and links to the web stay clickable. There is no print dialog and,
from the command line, no window you can see.

The export uses the system webview mbr already draws with (WebKit on macOS,
WebView2 on Windows, WebKitGTK on Linux), so it needs no extra software, and a
note's PDF matches what you see in mbr rather than a separate print engine's
idea of it.

## From the GUI

**File → Save as PDF…** (below **Print…**) asks where to save, suggests a file
name from the page title, and writes the PDF of the page on screen. A toast
says "Saving PDF…" and then "Saved *name*.pdf"; if anything went wrong, a
dialog says what and where.

| | macOS | Windows / Linux |
|-|-------|-----------------|
| Save as PDF… | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>P</kbd> |

(<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> is already Print on Windows and
Linux.) On Linux the shortcut works with the menu bar hidden, like every other
menu shortcut.

The page is exported in the background, in a hidden (on Windows, off-screen) view of its own: the window
you are reading is not touched, scrolled or re-rendered, and you can keep
reading while it saves. One PDF is saved at a time.

**Save as PDF… is greyed out while the [editor](editing/) is open.** The export
loads the note from disk, so it would silently leave out anything you have not
saved yet. Save (or close the editor) first; the item comes back as soon as the
editor closes.

The dialog opens in the folder you saved to last, else your Documents folder.
The GUI uses the [`[pdf]` settings](../reference/configuration/#pdf-export-settings)
of the repository that is open (paper, margin, page numbers, link base, timeout);
an edit to `.mbr/config.toml` applies to the next save.

## From the command line

```bash
mbr --pdf <NOTE> [--output <FILE|FOLDER|->] [--pdf-* options]
```

`--pdf` is a mode of its own, like `-b` or `-s`. It reads the repository's
configuration as `mbr -s` does, serves the note to itself on a private local
port, writes the PDF and exits. No window appears, nothing appears in the Dock or
taskbar, and focus stays where it was. A short note takes well under a second.
(On Windows the export's window is in fact shown while it renders, because
WebView2 may slow a hidden one down, but far off-screen, without a taskbar
button and without taking focus.)

```bash
# Writes "Meeting notes.pdf" in the current folder
mbr --pdf ~/notes/projects/Meeting\ notes.md

# A named file, or a folder (the PDF is named after the note)
mbr --pdf notes/report.md --output ~/Desktop/report-2026-11.pdf
mbr --pdf notes/report.md --output exports/

# To standard output: pipe it, upload it, attach it
mbr --pdf notes/report.md --output - > report.pdf
mbr --pdf notes/report.md --output - | curl -T - https://files.example.com/report.pdf

# A4, landscape, wide margins, page numbers
mbr --pdf notes/roadmap.md --pdf-paper a4 --pdf-landscape --pdf-margin 1in --pdf-page-numbers

# A custom sheet (millimetres or inches)
mbr --pdf notes/card.md --pdf-paper 148x210mm --pdf-margin 10mm

# Links between notes point at the published copy of the repository
mbr --pdf docs/guide.md --pdf-link-base https://notes.example.com/
```

The note can be any page mbr can open: a markdown file, or a folder (its index
note or listing; the PDF is named after the folder). Configuration, ignore
rules, the static folder, wikilinks and templates are the repository's own, so
links and images resolve as they do in the GUI.

Exporting a folder's worth of notes is a shell loop:

```bash
mkdir -p pdf
for note in notes/meetings/*.md; do
  mbr --pdf "$note" --output pdf/ || echo "failed: $note" >&2
done
```

The flags are in the [CLI reference](../reference/cli/#pdf-export); each one
beats the matching [`[pdf]` setting](../reference/configuration/#pdf-export-settings),
in either direction: `--pdf-portrait` and `--no-pdf-page-numbers` turn off a
`landscape` orientation or `page_numbers = true` set there.

**Output and exit status.** Only the PDF goes to standard output (and only with
`--output -`); errors and warnings go to standard error, and `-v` adds progress
lines there. The PDF is written to a temporary file beside the target, checked
(it must exist, be non-empty and start like a PDF) and only then renamed into
place, so a failed export never leaves a half-written or truncated file — and
never replaces an existing PDF with a broken one.

| Exit | Meaning |
|------|---------|
| `0` | The PDF was written (warnings, if any, on stderr) |
| `1` | The export ran and failed: the page could not be prepared, printing failed, or the written file could not be checked or moved into place. The message names the cause and the path |
| `2` | Refused before starting: a bad flag or value, a missing note, a configuration error, a note whose [page template](custom-page-templates/) cannot be used (missing, outside the repository, or failing to render: the message names the note, the template and why), an output folder that does not exist, is not a folder or is not writable, no display (Linux), or a platform without PDF support. Nothing was rendered |

## What ends up on paper

The PDF is the page as mbr prints it: only the note (the header, sidebar,
buttons and panels are left out), in light colours whatever your system
appearance, and with print rules applied — [FAQ answers](../markdown/definition-lists/)
shown, wide tables not cut at a scroll edge, kanban boards in one column. See
[Printing](../customization/themes/#printing) for those rules.

Before it prints, mbr waits for the page to finish: every image (lazy images
are loaded too), mermaid diagrams (drawn in their light colours), KaTeX math,
code highlighting and web fonts.

On macOS the export view is switched to the light appearance. On Windows and
Linux it is not (the appearance setting there is shared with mbr's own window,
which would turn light too); paper comes out light anyway, because mbr's
built-in dark colours apply only on screen and diagrams are drawn light for
export.

**Left out:** the [relationship charts](relationships/) below a person or
organization note. They are interactive views onto a larger graph (they load
as you scroll, animate, and open on the chart you last chose), none of which
makes a faithful page. The contact card itself, with its Relationships list, is
exported.

### Links

The export serves the note from a short-lived local address, so links to other
notes would be dead the moment the PDF is written. mbr therefore changes them:

| Link | In the PDF |
|------|------------|
| To another note or file in the repository | **Removed**, its text kept as plain text (the default) |
| … with a link base set | Rewritten onto it: `/guide/install/` becomes `https://notes.example.com/guide/install/` |
| To a heading on the same page (`#install`) | Kept |
| To the web, `mailto:`, `tel:` and other applications | Kept, clickable |
| [Out of the repository](links/#links-outside-the-folder) (`~/…`, `../../`) | Removed, text kept, with or without a link base |

Set a link base (`--pdf-link-base`, or `link_base` in `[pdf]`) when the
repository is also [published as a static site](../modes/build/): the PDF then
links into the published copy. The base must be an `http://` or `https://` URL;
a missing trailing `/` is added. Whether a same-page `#heading` link becomes a
clickable jump inside the PDF depends on the platform's PDF writer.

### Page numbers

Page numbers come from the platform's own header and footer, turned on with
`--pdf-page-numbers` or `page_numbers = true` (and off again for one export
with `--no-pdf-page-numbers`):

| Platform | With page numbers on |
|----------|----------------------|
| macOS | The page title and date at the top, "Page N of M" at the bottom, inside the margin |
| Windows | WebView2's header and footer, with the page title in the header and the page's address left out of the footer (it would name the export's short-lived local server). Not yet checked on a Windows machine |
| Linux | Nothing: WebKitGTK has no header or footer, so the setting is ignored (one line in the log, with `-v`) |

The format is fixed by the system. CSS page-margin boxes (`@top-center`,
`counter(page)`) are not supported by any of the three webviews, so a
stylesheet cannot add them either. Leave page numbers off for a PDF with no
header or footer at all.

### Paper and margins

Without a paper setting, mbr uses the system's: on macOS the default printer's
paper size, on Windows and Linux Letter in US and Canadian locales and A4
elsewhere. Named sizes are `letter`, `a4` and `legal`; anything else is a
custom `<width>x<height>` in `mm` or `in` (between 1 and 200 inches a side).
`--pdf-landscape` turns the sheet sideways; `--pdf-portrait` stands it upright
again when `[pdf]` says `orientation = "landscape"`.

The margin is a CSS `margin` shorthand — one to four lengths in `in`, `cm`,
`mm`, `pt`, `pc` or `px` (or `0`), top-right-bottom-left — applied to every page.
The default, `0.6in 0.5in`, is 0.6 inch top and bottom and 0.5 inch at the
sides.

## Customizing the PDF

Exported pages load one extra stylesheet, `export.css` (served at
`/.mbr/export.css`), after your theme. It is used **only** for PDF export — the
browser's own Print… never loads it — so it is the place for paper-only rules.
To change it, copy the default from mbr's source
(`crates/mbr-core/templates/export.css`) to `.mbr/export.css` in your repository
and edit it; like any `.mbr/` asset, your copy replaces the built-in one (see
[Customization](../customization/)).

The default does four things, all inside `@media print`: removes the theme's
print padding (the page margin is the `@page` margin, so they would add up),
removes body side padding, draws unlinked note links as plain text
(`a.mbr-export-unlinked`) and hides `<mbr-genealogy>`. For example, to add a
smaller font and keep code blocks off page breaks:

```css
/* .mbr/export.css — keep the default rules, then add yours */
@media print {
  main.container { padding: 0 !important; }
  body { padding-inline: 0 !important; }
  a.mbr-export-unlinked { color: inherit !important; text-decoration: none !important; }
  mbr-genealogy { display: none !important; }

  body { font-size: 11pt; }
  pre { break-inside: avoid; }
}
```

The sheet size and margin are not in this file: mbr writes them as an `@page`
rule into the page (`<style id="mbr-export-page">`) from the paper and margin
settings. Set the margin with `margin` in `[pdf]` or `--pdf-margin`, not with
`@page` here. Your `theme.css` and `user.css` apply to the export as they do to
printing.

## Linux without a display

On Linux the export needs a desktop session (X11 or Wayland), because the
webview it prints with does. Without one, `mbr --pdf` stops before starting,
with exit status 2:

```text
Error: PDF export needs a desktop session: neither DISPLAY nor WAYLAND_DISPLAY is set. On a server, run it under a virtual display (xvfb-run mbr --pdf …).
```

Headless servers and CI are not supported yet. Running under a virtual X
server usually works:

```bash
xvfb-run -a mbr --pdf notes/report.md --output report.pdf
```

Whatever makes the GUI run on your distribution (a working EGL/OpenGL driver
for WebKitGTK) is needed here too.

## Troubleshooting

**"the page did not finish rendering within 30.0 s; exported it as it was".**
mbr waits up to the timeout (`--pdf-timeout-ms`, `timeout_ms` in `[pdf]`,
default 30 seconds) for the page to say it is ready, then exports what it has,
with this warning. A huge note, or an image on a slow server, can need longer:
raise the timeout. From the command line, the repository scan (for wikilinks,
backlinks and the site index) gets the same budget first; if it does not finish,
mbr warns and exports without it.

**"N image(s) still loading after 10 s: …".** Images get ten seconds between
them. One that is still loading then is printed as it is (usually blank) and
named in a warning; one that failed is named too. The PDF is still written.
Diagrams, math, code highlighting and fonts each get fifteen seconds, with the
same kind of warning. Warnings never fail the export: from the command line they
are printed to stderr with exit status 0; in the GUI the toast shows the first
and the log has the rest.

**"The folder for the PDF does not exist" / "Cannot write to the folder".**
mbr checks the output folder before rendering anything, by creating its
temporary file there, and stops with exit status 2. Create the folder, or pick
one you can write to. A `--output` ending in `/` always means a folder.

**"No PDF was written" / "is not a PDF".** The system's print reported success
but left no usable file (macOS does this for a folder removed during the
export). Nothing was put in place; try again.

**Raw diagram source or TeX in the PDF.** The page's components did not load —
usually a custom `.mbr/components/` override that fails. Run with `-v` and
check stderr for warnings about diagrams or math.

**The PDF is dark, or the colours are off.** Exports print light colours. If
yours does not, a custom `theme.css` probably sets dark colours outside
`@media screen` (on Windows and Linux, a dark system appearance then reaches
the paper, since the export view is not switched to light there); see
[Printing](../customization/themes/#printing).

**Save as PDF… is greyed out.** The editor is open (save or close it), or
this platform has no PDF export yet.
