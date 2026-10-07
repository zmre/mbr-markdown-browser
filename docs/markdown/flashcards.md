---
title: Flashcards
description: Turn a note's definition lists into a flashcard deck, with optional spaced repetition
type: flashcard
order: 8
---

# Flashcards

Give a note `type: flashcard` in its frontmatter and every definition list in it
becomes a deck: each term is the front of a card, its definitions are the back.
A **Review flashcards** button appears in the header — or press `p` — and the
cards open full screen, one at a time, to flip through in order, at random, or
with [spaced repetition](#spaced-repetition--fsrs).

**This page is itself a deck.** The [cards at the bottom](#the-demo-deck) are
real: press `p` to try them.

## Why definition lists?

We wanted to be compatible with an established markdown flashcard format, and
looked at [Neuracache](https://neuracache.com/markdown-flashcards),
[kanad13/markdown-flashcards](https://github.com/kanad13/markdown-flashcards),
[bttger/markdown-flashcards](https://github.com/bttger/markdown-flashcards) and
[Mochi](https://mochi.cards/docs/getting-started/create-a-card/). None was a good
model: they want a note per flashcard, or HTML comments, or special one-off
syntax.

The core requirement is that notes with flashcards stay **nicely readable as
text** and **render nicely in most places by default** — in an editor, on
GitHub, in other markdown tools — with anything extra being enhanced styling
rather than syntax that mucks up the text. A definition list is exactly that:
a term followed by its definitions is already "a question and its answer", it
is supported by most markdown renderers, and it reads naturally as plain text.
mbr only adds behaviour on top.

## Writing cards

```markdown
---
type: flashcard
---

What is the capital of France?
: Paris.

Which two fields set a page's body classes?
: `style`
: `type`
```

- The term is the front; every `:` line under it is part of the back. Several
  answers are shown together, one after another.
- Answers can hold anything markdown can: lists, code blocks, images, math,
  links. Indent continuation lines to line up with the answer's text.
- **Leave a blank line before the next term.** Without one, the next question
  is swallowed into the previous answer as a lazy continuation — the same
  [definition-list gotcha](./#leave-a-blank-line-between-entries) as anywhere
  else.
- Only *top-level* definition lists are cards. A list nested inside a block
  quote or a list item is left out of the deck.
- Tight (no blank lines) and loose (blank line between the term and its `:`
  lines) lists both work.

In the page itself, a flashcard note keeps mbr's usual
[FAQ-style](./#definition-lists--faq-style) definition lists: the answers stay
hidden until you click a question, which is a natural way to quiz yourself
without opening the deck at all.

## Reviewing

The overlay shows one card at a time, sized to fill the screen: the text is set
as large as it can be while still fitting (and scrolls once it would be too
small to read). Click the card, or press `Enter`, to flip it.

The top bar has the progress counter, a **Swap sides** toggle (show the answer
first and recall the question), the mode, and the close button.

| Mode | Order | On the back of a card |
|------|-------|-----------------------|
| In order | As written | **Next** / **Previous** |
| Random | Shuffled each time | **Next** / **Previous** |
| Spaced repetition (FSRS) | Due cards first, then new ones | Rate it: **1 Again**, **2 Hard**, **3 Good**, **4 Easy** |

Spaced repetition is the default when it is available (see below); otherwise
the deck opens in Random order.

### Keys

The navigation keys mirror the ones [slides](slides/) use, so the same fingers
work in both:

| Key | Action |
|-----|--------|
| `p` | Open the deck (on a flashcard page) |
| `Space` / `→` / `PageDown` / `n` | Advance: flip a card that shows its front; on the back, go to the next card |
| `Enter` / click | Flip the card either way |
| `←` / `PageUp` | Previous card (In order / Random) |
| `Home` / `End` | First / last card (In order / Random) |
| `1` `2` `3` `4` | Rate Again / Hard / Good / Easy (spaced repetition, on the back) |
| `Esc` | Leave the deck and return to the page |

In spaced-repetition mode a card can only be left by rating it, so on the back
`Space` and `→` do nothing except point at the rating buttons — a stray key
cannot skip a card you were meant to grade. Rating a card (by key or click)
saves it and moves straight on to the next.

## Spaced repetition (FSRS)

Spaced repetition shows each card again just before you would forget it, so
easy cards come back rarely and hard ones often. mbr schedules with
[FSRS](https://github.com/open-spaced-repetition/fsrs4anki/wiki), the Free
Spaced Repetition Scheduler that modern Anki uses, through the
[ts-fsrs](https://github.com/open-spaced-repetition/ts-fsrs) library. The four
ratings mean what they mean in Anki:

| Rating | Meaning |
|--------|---------|
| Again | You did not remember it |
| Hard | You remembered, with real effort |
| Good | You remembered after a moment's thought |
| Easy | You remembered instantly |

Each button shows when the card would come back, e.g. `1m`, `10m`, `3d`.

A session shows the cards that are **due** first — the ones you are most likely
to have forgotten first — and then the cards you have never reviewed, in
document order. A card you rate Again (or a new card still in its first
minutes-long learning steps) is shown again later in the same session rather
than in ten minutes' time, like Anki's "learn ahead". When nothing is due, the
deck says when the next card is, and offers to review at random anyway.

Scheduling uses FSRS's default parameters (90% target retention, 1-minute and
10-minute learning steps, a 10-minute relearning step) with **fuzz off**, so the
same history always produces the same schedule.

### When it is available

Spaced repetition has to write your results somewhere, and mbr writes them into
the note itself — so it is offered only when the page is served with
[editing enabled](../modes/editing/) (`mbr -s` / `mbr -g` with editing on). A
static site, or a server without editing, still offers In order and Random. On
this documentation site, which is a static build, the mode is not offered.

The deck finds each card in the file by its question's source line, which the
page carries as `data-mbr-line`. That attribute also serves
[review notes](../reference/configuration.md#review-settings), but turning those
off with `--no-review` does not take spaced repetition with it: with editing on,
questions keep their line and nothing else on the page gets one.

### The review history

Each rating is appended to the card as one more definition:

```markdown
What is the capital of France?
: Paris.
: ___Review History___
  * 2026-10-06 13:45 - Again
  * 2026-10-06 13:55 - Good
  * 2026-10-07 09:10 - Hard
  * 2026-10-09 18:02 - Easy
```

- The history is the definition whose text starts with the emphasised words
  *Review History* (`___Review History___` is what mbr writes; `**Review
  History**` and `*Review History*` are recognised too, in any case). It is
  never shown as part of the answer.
- Each line is `YYYY-MM-DD HH:MM - Rating`, with the rating one of `Again`,
  `Hard`, `Good` or `Easy`. `Fail` is accepted as another word for `Again`, for
  history written by other tools; mbr always writes `Again`. Lines in any other
  shape are ignored by the scheduler and left untouched.
- Nothing else is stored. Every time the deck opens, each card's FSRS state is
  recomputed by replaying its history, so editing or deleting history lines by
  hand is perfectly fine.
- In the page, the history collapses to a single line such as
  *Reviewed 4× · last: Easy, Oct 9*; click it to see the entries. (In the deck,
  **History** on the back of a card shows the entries and the card's current
  due date, stability and difficulty.)

Each entry is stamped with your device's clock when you rate the card, and the
server writes only that line (plus the `___Review History___` line the first
time), keeping the file's line endings.
If the note changed on disk since the page loaded, nothing is written and the
deck says so; reload the page to continue. The endpoint is
[`POST /.mbr/flashcard-review`](../modes/editing/#flashcard-reviews).

> [!NOTE]
> Times are your local wall-clock time, without an offset, and are read back in
> the browser's time zone — so it does not matter where the server runs. The
> server refuses a time more than a day or so from its own clock, which catches
> a badly set device clock. Reviewing while travelling across time zones writes
> entries in whichever zone you are in, which can nudge a day boundary slightly;
> that is the price of history a human can read at a glance.

## The demo deck

A small deck about mbr, markdown and general knowledge. Press `p`, or click a
question to peek at its answer.

What does **mbr** stand for?
: Markdown browser.

Which key opens search in mbr?
: `/` (slash).

How do you write a markdown link to example.com with the text "Example"?
: ```markdown
  [Example](https://example.com)
  ```

Which two frontmatter fields set a page's `<body>` classes?
: `style`
: `type`

What are mbr's three main jobs?
: - A **previewer** that renders markdown on the fly
  - A **browser** for navigating, searching and tagging a whole repository
  - A **static site generator**

What does the mbr logo look like?
: ![The mbr logo](../images/logo.png)

What does FSRS stand for?
: Free Spaced Repetition Scheduler.

What is the capital of Australia?
: Canberra.
: ___Review History___
  * 2026-09-14 08:12 - Again
  * 2026-09-14 08:24 - Good
  * 2026-09-17 19:40 - Good

What is the chemical symbol for gold?
: Au, from the Latin *aurum*.
: ___Review History___
  * 2026-09-20 21:03 - Fail
  * 2026-09-20 21:15 - Hard

Which planet has the shortest day?
: Jupiter — it turns once in about 9 hours and 56 minutes.

How do you start a fenced code block in markdown?
: A line of three backticks (or three tildes), optionally followed by a
  language name.

In a definition list, what does a line starting with `:` mean?
: It is a definition — on a flashcard page, an answer — of the term above it.

How many bits are in a byte?
: Eight.
