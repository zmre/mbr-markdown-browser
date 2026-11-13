---
title: Copying Text and Pages
description: Copy a selection or the whole note as its markdown source or as rich text, without mbr's own buttons and markers
order: 16
---

# Copying Text and Pages

**Works in:** GUI · Server · Static site · iOS

Select text in a note and a small toolbar appears beside the selection:

- **Copy markdown** puts the note's own markdown for the selection on the
  clipboard — what you typed, not what mbr drew. A selection inside one
  paragraph, heading, list item or code block copies just the matching part of
  its source, with any emphasis, code span or link it touches kept whole
  (select a word of a link and you get the link). A selection across several
  blocks copies those blocks' lines whole, list markers, `>` and all. When mbr
  cannot find the selected words in the block's source (text drawn from a task
  date, say), it copies the whole block.
- **Copy rich** puts the formatted HTML on the clipboard, so a paste into a mail
  client, a word processor or a chat keeps headings, emphasis, lists, tables
  and links. Plain text goes along with it, for applications that only take
  text.

When [review notes](review/) are on, the same toolbar carries **Add note**.

To copy the **whole note**, open the info panel (<kbd>Ctrl</kbd>+<kbd>G</kbd>)
and use **Copy page: Markdown** or **Rich**, between the Metadata and Document
Info sections. In the GUI the **Edit** menu has the same two commands, **Copy
Page as Markdown** and **Copy Page as Rich Text**. A markdown copy of the page
is the note's body without its frontmatter. A rich copy is everything in the
page body — the title, a contact's card, the text — but not the header, the
sidebar or a periodic note's previous/next bar.

## When the button says "Copy text"

Markdown comes from the note's file, which the page reads from the server, and
a selection is matched to the file by the source lines mbr marks on each block.
Where either is missing the button reads **Copy text** (**Text** in the info
panel) and copies plain text instead: paragraphs and headings separated by a
blank line, list items and table rows one per line, table cells separated by
tabs, code blocks verbatim. That happens:

- on a **static site** (there is no server to read the file from);
- on a server started **without editing** (`mbr -s` without `--edit`): the
  file is read through the editing endpoint. The GUI has editing on;
- for a selection on a server started with **`--no-review`**, which drops the
  source lines from the page (the whole page still copies as markdown);
- for a selection that takes in text the file does not hold, such as a
  contact's card.

If reading the file fails when you copy (a server that wants an
[edit token](editing/) you have not entered, say), the copy falls
back to text and the message says "Copied as text".

## What is left out

A copy never carries the things mbr adds to a page for the reader: heading
permalinks (`#`), review-note markers, the icons after external links,
flashcard progress pies, buttons and other controls, element ids and mbr's
`data-mbr-*` attributes. In a rich copy, relative links and image addresses are
made absolute, so a link in the pasted text still leads to the page it pointed
at. A markdown copy is the file's text as written, relative links included.

## When copying fails

A short message says whether the copy worked. Browsers only let a page write
the clipboard in answer to a click or a key, and a page served over plain
`http` to another machine (`mbr -s --host 0.0.0.0`, reached by IP) gets the
older clipboard route only; if neither works, "Copy failed" is shown and the
browser's own **Copy** still works on a selection.
