---
title: Kanban Display
description: Show lists in trello-like kanban-style columns
order: 5
---

# Kanban Display

You can make sections display as trello-like kanban columns where each bullet in a list is a card.  You can also do this for an entire file by adding `style: kanban` to the frontmatter, but to do it just for a section in a document, use the `--- {.kanban}` divider to create a section with a kanban style.  Here's a quick example:

```markdown
--- {.kanban}

## Backlog

* Semantic search (or hybrid)
  * Local
  * Client-side
* Browser tag filters
* Knowledge cluster visual browsing
  * Via tags, semantic meanings, folders, links (in/out), etc.

## On deck

* Upgrade to pagefind 1.5 when released
* Add benchmarks to track performance
  * [x] Explorer to understand changes over time
  * [x] Static build and live previewer
  * [ ] Automated benchmarking on release via CI
* Small text variant for slides
* Video demos

## In progress

* Kanban display
  * Pure CSS
  * Existing syntax -- no new parsing or HTML
* Something else
  * Lorem ipsum dolor sit amet, consectetur adipisicing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.

## Completed

* Quicklook previews

---
```

This is just for display -- if you want to move items, do it in the editor.  Here's what the above markdown renders like (note: you may need to scroll horizontally):

--- {.kanban}

## Backlog

* Semantic search (or hybrid)
  * Local
  * Client-side
* Browser tag filters
* Knowledge cluster visual browsing
  * Via tags, semantic meanings, folders, links (in/out), etc.

## On deck

* Upgrade to pagefind 1.5 when released
* Add benchmarks to track performance
  * [x] Explorer to understand changes over time
  * [x] Static build and live previewer
  * [ ] Automated benchmarking on release via CI
* Small text variant for slides
* Video demos

## In progress

* Kanban display
  * Pure CSS
  * Existing syntax -- no new parsing or HTML
* Something else
  * Lorem ipsum dolor sit amet, consectetur adipisicing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.

## Completed

* Quicklook previews

---

This is useful for tracking projects, issues, tasks, etc., and seeing them in a more visual way.  It keeps the markdown pretty standard (particularly if your whole doc is the kanban and you just add the `style: kanban` bit to the frontmatter).

> [!NOTE]
> This uses CSS columns and sometimes columns wrap despite being instructed not to.
