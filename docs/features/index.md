---
title: Features
description: What mbr does with your notes — browsing, search, links, tags, calendar, tasks, slides, flashcards, contacts, editing and more
order: 4
---

# Features

These pages are about what mbr **does** with a folder of markdown: how you
find your way around it, and the views and tools it builds from your notes.
The syntax they read is in [Markdown Syntax](../markdown/); most features need
no syntax beyond ordinary markdown and a line of frontmatter.

## Finding your way

| Feature | What it is |
|---------|------------|
| [Browsing](browsing/) | The file browser sidebar (`-`), folders, recent files, pinned shortcuts, breadcrumbs, next/previous |
| [Search](search/) | Full-text and frontmatter search (`/`), by folder, type or date |
| [Links, Backlinks and the Info Panel](links/) | Links out and in (`f` / `F`), table of contents (`T`), the info panel and its link graph (`Ctrl+G`) |
| [Tags](tags/) | Tag pages and the tag tree, from any frontmatter field |
| [Calendar](calendar/) | Daily, weekly and monthly notes, the day companion, the calendar overlay (`c`) |

## Views of your notes

| Feature | What it is |
|---------|------------|
| [Task Browser](tasks/) | Every task in the repository, filtered and grouped, by note or by due date (`t`) |
| [Slides](slides/) | Present a note as a slide deck (`p`); see the [example deck](slides-example/) |
| [Flashcards](flashcards/) | Practise a note's definition lists, with optional spaced repetition |
| [Kanban](kanban/) | Show lists as columns of cards |
| [Page Styles and Types](page-styles/) | Restyle a whole page from frontmatter |
| [Contacts](contacts/) | People and organizations as notes, with a contact card |
| [Relationships](relationships/) | Typed links between notes, family trees and org charts |

## Writing

| Feature | What it is |
|---------|------------|
| [In-Browser Editing](editing/) | Edit, create, rename and move notes from the page (`e`) |
| [Editing Properties](editing-properties/) | Frontmatter as a form or as YAML, with suggestions from your notes |
| [Review Notes](review/) | Annotate a page and copy the whole review out as markdown (`r`) |

## Where each feature works

| Feature | GUI | Server | Static site | iOS | QuickLook |
|---------|:---:|:------:|:-----------:|:---:|:---------:|
| Browsing, search, links, tags | ✓ | ✓ | ✓ | ✓ | |
| Calendar and day companion | ✓ | ✓ | ✓ | ✓ | day calendar |
| Task browser | ✓ | ✓ | | ✓ | |
| Slides, kanban, page styles | ✓ | ✓ | ✓ | ✓ | |
| Flashcards | ✓ | ✓ | ✓ (no spaced repetition) | ✓ | |
| Contacts and relationship charts | ✓ | ✓ | ✓ | ✓ | contact card |
| Editing and properties | ✓ (on by default) | with `--edit` | | ✓ | |
| Review notes | ✓ | ✓ | | with a keyboard | |

The modes themselves are described in [Modes of Operation](../modes/).
