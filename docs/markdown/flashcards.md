---
title: Flashcards
description: Turn a note's definition lists into a flashcard deck, with Concentric practice and optional spaced repetition
type: flashcard
order: 8
---

# Flashcards

Give a note `type: flashcard` in its frontmatter and every definition list in it
becomes a deck: each term is the front of a card, its definitions are the back.
A **Review flashcards** button appears in the header — or press `p` — and the
cards open full screen, one at a time, to flip through in order, at random, in a
growing [Concentric (FSRS)](#concentric--fsrs) stack, or with
[spaced repetition](#spaced-repetition--fsrs).

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
  else. (The one exception is an answer ending in a code block, which has no
  paragraph to continue. A question right after one is a separate card, and
  when mbr records a review above it, it adds a blank line so the question
  stays separate.)
- Only *top-level* definition lists are cards. A list nested inside a block
  quote or a list item is left out of the deck.
- Tight (no blank lines) and loose (blank line between the term and its `:`
  lines) lists both work.

### Multi-line questions

A question can span several consecutive lines, but markdown joins them into
one run of text. To keep each line on its own line, end it with a backslash
(`\`). A line holding only a backslash gives you a blank line inside the
question, which is handy for setting multiple-choice options apart from the
question itself:

```markdown
Which of the following is part of the Basis and Purpose of the Amateur Radio Service?\
\
A. Providing personal radio communications for as many citizens as possible\
B. Providing communications for international contesting\
C. Advancing skills in the technical and communication phases of the radio art\
D. All these choices are correct
: D
```

Don't use a truly empty line instead. A blank line ends the question: the
lines above it become an ordinary paragraph outside the deck, and only the
lines after it become the card's front. A question can't hold real blocks such
as a markdown list either — only answers can.

In the page itself, a flashcard note keeps mbr's usual
[FAQ-style](./#definition-lists--faq-style) definition lists: the answers stay
hidden until you click a question, which is a natural way to quiz yourself
without opening the deck at all.

## Reviewing

The overlay shows one card at a time, sized to fill the screen: the text is set
as large as it can be while still fitting (and scrolls once it would be too
small to read). Click the card, or press `Enter`, to flip it.

The top bar has the progress counter, a **Swap sides** toggle (show the answer
first and recall the question), the [section filter](#filtering-by-section)
when the note has headings, the **Cards** box in Concentric mode, the mode, and
the close button.

| Mode | Order | On the back of a card | Needs editing on |
|------|-------|-----------------------|------------------|
| In order | As written | **Next** / **Previous** | No |
| Random | Shuffled each time | **Next** / **Previous** | No |
| Concentric (FSRS) | A small stack that grows as you learn it | Rate it: **1 Again**, **2 Hard**, **3 Good**, **4 Easy** | No (ratings are saved only when it is) |
| Spaced repetition (FSRS) | Due cards first, then new ones in random order | Rate it: **1 Again**, **2 Hard**, **3 Good**, **4 Easy** | Yes |

The deck always opens in Concentric (FSRS), with editing on or off; pick another
mode from the menu or with its key (below).

### Keys

Everything in the deck works from the keyboard. Press `?` in the deck for a
list of every key; the hint line at the bottom says so. The navigation keys
mirror the ones [slides](slides/) use, so the same fingers work in both.

**Navigation**

| Key | Action |
|-----|--------|
| `p` | Open the deck (on a flashcard page) |
| `Space` / `→` / `PageDown` / `n` | Advance: flip a card that shows its front; on the back, go to the next card |
| `Enter` / click | Flip the card either way |
| `←` / `PageUp` | Previous card (In order / Random) |
| `Home` / `End` | First / last card (In order / Random) |

**Rating** (Concentric and spaced repetition, on the back of a card)

| Key | Action |
|-----|--------|
| `1` | Again |
| `2` | Hard |
| `3` | Good |
| `4` | Easy |

**Deck controls**

| Key | Action |
|-----|--------|
| `c` | Switch to Concentric (FSRS) |
| `o` | Switch to In order |
| `r` | Switch to Random |
| `s` | Switch to Spaced repetition (FSRS); ignored when it is not offered |
| `w` | Swap sides (show the answer first) |
| `f` | Open the [section filter](#filtering-by-section) with focus on its first box (when the note has one). Inside it: `↑` / `↓` (or `Tab`) move, `Space` ticks, `a` picks **All sections**, `Esc` or `f` closes it and puts focus back |
| `+` / `=` | One more card in play (Concentric; same as raising the **Cards** box) |
| `-` | One fewer card in play (Concentric) |
| `#` | Put the cursor in the **Cards** box (Concentric). Type a number; `Enter` applies it, `Esc` leaves it unchanged, and both return to the card |
| `?` | Show or hide the list of shortcuts (`Esc` also closes it) |
| `Esc` | Close the shortcut list, the **Cards** box or the section filter if one is open; otherwise leave the deck and return to the page |

A mode key for the mode you are already in does nothing, so it cannot lose your
place. While the deck is open the page's own shortcuts are switched off, and keys
held with `Cmd`, `Ctrl` or `Alt` are left alone, so browser shortcuts such as
printing still work.

In the rated modes (Concentric and spaced repetition) a card can only be left
by rating it, so on the back `Space` and `→` do nothing except point at the
rating buttons — a stray key cannot skip a card you were meant to grade, and
there is no going back. Rating a card (by key or click) moves straight on to
the next. Keys typed into the **Cards** box or the filter stay there.

## Concentric (FSRS)

Concentric (FSRS), the deck's default mode, drills a **small stack** of cards
until you know it, then folds in a couple more — so the stack grows outward in
rings, like concentric circles. It is closely related to *Incremental Rehearsal*, also called
*folding-in* ([Tucker 1988](https://doi.org/10.13140/RG.2.2.13505.56164),
unpublished teaching materials, often cited as Tucker 1989), a drill technique
from special education where unknown items are worked one at a time into a set
that is mostly known. A meta-analysis of 19 Incremental Rehearsal studies by
Burns, Zaslofsky, Kanive and Parker found large effects on learning, though no
clear efficiency advantage over other drills
([Burns et al. 2012](https://doi.org/10.1007/s10864-012-9160-2);
[overview at Intervention Central](https://www.interventioncentral.org/academic-interventions/math-facts/math-computation-promote-mastery-math-facts-through-incremental-re)).
It needs no editing: it works on a static site too.

How it runs:

1. **Starting stack.** Six cards (or every card, if the note has fewer). If the
   note has any review history, a quarter of the stack, rounded up, is seeded
   with *challenging* cards: first ones whose latest rating was Again or Hard,
   then ones never reviewed. The rest are random. A note with no history at
   all starts with a plain random six.
2. **Rate each card.** The card goes back into the stack at a position that
   depends on the rating, with a little jitter:

   | Rating | Goes back… |
   |--------|-----------|
   | Again | about a quarter of the way in (20–35%), but never as the very next card |
   | Hard | about halfway (40–60%) |
   | Good | near the back (75–100%) |
   | Easy | to the very back |

3. **Grow.** A *pass* is as many ratings as there are cards in the stack. At the
   end of a pass, if at least 70% of the cards rated in it got Good or Easy *the
   first time they came up in that pass*, two new cards are folded in at random
   places. Only first answers count, so hammering one card until it is Good
   cannot make the stack look learned. Because each pass judges whole cards,
   the bar moves in steps: with six cards, 70% means five of six (four of six is
   67%); with eight, six of eight.
4. **Mastered.** Once every card is in the stack and a pass clears the bar, the
   deck says so and offers **Keep practising** (the same stack carries on) or a
   new stack. Otherwise Concentric never ends on its own — close it whenever you
   like.

The **Cards** box in the top bar shows the stack's size and follows it as it
grows. Change it at any time: raising it adds cards (topping the challenging
quarter up first, when the note has history), lowering it sends the cards you
know best back to the pool (Easy before Good before not yet rated, never the
card on screen). The counter reads `8 in play · 4 left`: cards in the stack,
and eligible cards not yet folded in.

**Saving.** With [editing enabled](../modes/editing/), every Concentric (FSRS) rating is
written to the card's [review history](#the-review-history) exactly as a spaced
repetition rating is, so practice counts toward FSRS scheduling later. Without
editing — a static site, a read-only server — ratings last for the session only:
they steer the stack but nothing is written. If a write fails because the note
changed on disk, the deck says so and carries on session-only.

The growth threshold is configurable with
[`flashcards_concentric_threshold`](../reference/configuration.md#flashcard-settings)
(default `0.7`).

### Where the numbers come from

- **Six cards to start.** Working memory holds only about four chunks at once
  ([Cowan 2001](https://doi.org/10.1017/S0140525X01003922)); six keeps the challenging
  cards few enough to hold in mind while the rest of the stack, mostly known,
  gives each of them a gap between repetitions.
- **A 70% bar.** Incremental Rehearsal works by keeping most of the drill set
  known, with only a few unknown items in it at a time, and a meta-analysis of
  drill ratios found strong effects whenever at least half the items were known
  ([Burns 2004](https://doi.org/10.1177/07419325040250030401)); growing only
  after most of a pass was answered well keeps the stack in that range. For
  comparison, a theoretical model of learning puts the optimal success rate near
  85% ([Wilson et al. 2019](https://doi.org/10.1038/s41467-019-12552-4), a
  result derived for gradient-descent learners on binary classification tasks
  rather than measured in a classroom).
  70% is a little looser, so a stack grows steadily instead of stalling on one
  stubborn card.
- **Grow by two.** A small stack spaces its repetitions too closely: drilling a
  few cards over and over masses their practice, and one large stack, with more
  cards between each repetition, beats several small ones
  ([Kornell 2009](https://doi.org/10.1002/acp.1537)).
  So the stack has to grow, but two at a time keeps the share of new cards
  small, as folding-in does.
- **Where a card goes back.** Delaying the first retrieval matters more than
  how later ones are spaced
  ([Karpicke & Roediger 2007](https://doi.org/10.1037/0278-7393.33.4.704)), so a
  card you missed is never shown again straight away. A harder retrieval at a
  longer lag strengthens memory more, provided it still succeeds
  ([Pyc & Rawson 2009](https://doi.org/10.1016/j.jml.2009.01.004)): Again
  comes back soon enough to be recalled, Hard waits longer, and Good and Easy
  wait longest.
- **A quarter challenging.** Seeding a quarter of the stack with cards you
  struggled with (or have never seen) makes sure they get practice, while
  three quarters stay cards you are likely to know. That is looser than
  Incremental Rehearsal's seven to nine known items per unknown, but within the
  range of mostly-known ratios that worked well in Burns's 2004 meta-analysis.

## Filtering by section

When the note's cards sit under two or more headings, the top bar has a funnel
button. It opens a list of the headings that contain cards, indented by level;
tick one or more to review only the cards under them. A heading includes
everything under it, subheadings too. With nothing ticked (**All sections**),
every card is in the deck, and the funnel shows a dot while a filter is on.
A heading that holds every card in the note is left out, since it would
filter nothing.

Changing the filter restarts In order and Random on the chosen cards, re-plans a
spaced-repetition session over them, and in Concentric swaps cards in and out
of the stack while keeping its size. `Esc` or a click elsewhere closes the
list. The filter is not remembered: each time the deck opens it starts on every
card.

## Progress in the page

Once a note has any review history, the page itself shows how it is going:

- **Each reviewed question** gets a thin left border in the colour of its latest
  rating (red Again, amber Hard, green Good, blue Easy). Hover it to see when:
  *Last review: Easy, Oct 9*.
- **Each heading with cards under it** gets a small pie chart of its cards'
  latest ratings, with never-reviewed cards as a grey slice. The note's title
  shows the whole note. Hover a pie for the breakdown, e.g. *Easy 40% · Good 20%
  · Hard 10% · Again 20% · Not reviewed 10% (10 cards)*.

The pies are drawn without any text, so selecting or searching a heading finds
only its words. After a deck session that saved reviews, the borders and pies
redraw when the deck closes. A note with no history shows neither. Turn them
off with
[`flashcards_progress_indicators = false`](../reference/configuration.md#flashcard-settings).

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
to have forgotten first, with ties broken at random — and then the cards you
have never reviewed, **shuffled**, so a note with little history does not open
on its first card every time. A card you rate Again (or a new card still in its first
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
static site, or a server without editing, still offers Concentric (FSRS) —
session-only, nothing saved — In order and Random. On
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
question to peek at its answer. The two headings below are there to try the
[section filter](#filtering-by-section) and the [progress pies](#progress-in-the-page).

### mbr and markdown

What does **mbr** stand for?
: Markdown browser.
: ___Review History___
  * 2026-10-06 20:59 - Easy

Which key opens search in mbr?
: `/` (slash).
: ___Review History___
  * 2026-10-06 20:59 - Good

How do you write a markdown link to example.com with the text "Example"?
: ```markdown
  [Example](https://example.com)
  ```
: ___Review History___
  * 2026-10-06 21:00 - Easy

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

How do you start a fenced code block in markdown?
: A line of three backticks (or three tildes), optionally followed by a
  language name.

In a definition list, what does a line starting with `:` mean?
: It is a definition — on a flashcard page, an answer — of the term above it.

### General knowledge

What is the capital of Australia?
: Canberra.
: ___Review History___
  * 2026-09-14 08:12 - Again
  * 2026-09-14 08:24 - Good
  * 2026-09-17 19:40 - Good
  * 2026-10-06 20:59 - Again

What is the chemical symbol for gold?
: Au, from the Latin *aurum*.
: ___Review History___
  * 2026-09-20 21:03 - Fail
  * 2026-09-20 21:15 - Hard
  * 2026-10-06 20:59 - Easy

Which planet has the shortest day?
: Jupiter — it turns once in about 9 hours and 56 minutes.

How many bits are in a byte?
: Eight.
