---
title: Definition Lists (FAQ)
description: Term and definition pairs rendered as a click-to-reveal FAQ
order: 10
---

# Definition Lists (FAQ)

Definition lists render as an FAQ. Each term becomes a question you can open,
and its definition stays collapsed until you do:

```markdown
What is mbr?
: A markdown browser, previewer and static site generator.

Does this need JavaScript?
: No. The expand and collapse are pure CSS.
```

## Live Example

What is mbr?
: A markdown browser, previewer and static site generator. Click the question
  above to toggle this answer.

Where do I read more?
: Start with the [Quickstart](../getting-started/quickstart/), then the
  [CLI Reference](../reference/cli/).

Can one question have several answers?
: Yes — repeat the `:` line.
: Every answer of that question opens together.

## Leave a Blank Line Between Entries

This is the one gotcha worth knowing. Without a blank line, the next term is
swallowed as a lazy continuation of the previous definition:

```markdown
First question?
: First answer.
Second question?          <-- becomes part of the FIRST answer's text
: Second answer.
```

Written that way you get **one** question with three answers instead of three
questions. Separate every entry with a blank line:

```markdown
First question?
: First answer.

Second question?
: Second answer.
```

An empty answer (a `:` with nothing after it) followed by a blank line ends
the whole list, as an empty list item does: the next question starts a new
list, and a `:` line right after the blank line is plain text.

## Behavior

- **Click a question, or Tab to it,** to reveal its answer. For keyboard users
  focus alone opens it — there is nothing to press.
- **Tab again** to reach links inside the answer. The answer stays open while
  focus is anywhere inside it.
- **Links inside a closed answer are skipped by Tab** and hidden from screen
  readers, so you never land on content you cannot see.
- A question that owns **several answers** opens all of them at once.

## Accepted Limitations

The disclosure is CSS `:focus` with no JavaScript and no `<details>` element,
which buys two rough edges:

- **Clicking an open question does not close it.** The click only re-focuses a
  question that already has focus, and CSS cannot toggle. Clicking a *different*
  question, or anywhere off the list, does close it.
- **Clicking inside an open answer closes it,** because focus moves to the page
  body. This mostly bites when selecting an answer's text; start the selection
  from the end of the question instead, or print the page.

If you need a real toggle, write literal `<details>`/`<summary>` HTML in your
markdown instead, or override the rules in `.mbr/theme.css`.

## Printing

Print styles force every answer open and drop the disclosure markers, so a
printed page or PDF export shows the complete Q&A rather than a list of
unanswered questions. The same goes for the rest of the page: collapsed heading
sections, clipped link cards and closed `<details>` blocks are all expanded
before printing — see [Printing](../customization/themes/#printing) for the
full list and for the browser-version caveat on `<details>`.

## Restyling

Colors, spacing, the marker and the animation are all driven by `--mbr-dl-*`
custom properties — see
[Definition Lists in CSS Theming](../customization/themes/#definition-lists--faq).
