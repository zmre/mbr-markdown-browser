# Tasks Spec

We already support tasks in markdown files for display.  We also support live searches via grep which we could adapt to find all tasks in a given folder hierarchy (looking for any valid commonmark task plus ones that fit our extension).  Our current markdown supports a slightly extended syntax for tasks that allows canceling via `-`.  

The goal of this feature is to add the ability to search and explore tasks in a given markdown repo (or any repo with markdown files). In the future, we may support a list of regex's to also look for so we can pick up things like TODO: and FIXME: in code bases, but that's out of scope for our initial implementation.

## Extended Syntax

Right now we support standard commonmark task syntax and additionally support canceled tasks that look like this:

```markdown
* [-] this task was canceled
```

But in other contexts, I use some other annotations on tasks that we'll want some sort of support for.  In particular, at the end of a task line I can add `@done(YYYY-MM-DD HH:MM)` and `@due(YYYY-MM-DD HH:MM)` annotations. I also use tags sometimes, which are restricted to one word (dashes and underscores okay, spaces not) and are on a task line like this: `#mytag`. And I support priorities with ` !! ` indicating high priority and ` !!! ` indicating urgent. There's no low priority; the default is a normal priority.

Put together, we can have tasks like:

```markdown
* [ ] this is a task due tomorrow @due(2026-08-05)
* [x] this task was done yesterday @done(2026-08-04 12:11 PM)
* [ ] this task is urgent !!! #hotlist #work @due(2026-08-04 03:00 PM)
```

## GUI

We need a popup that appears on a keypress (i think lowercase `t` is free?) or when a clipboard icon in the header is clicked.  This popup (which can be disabled via config), is a two-pane display.  On the left is a nested folders list like in our browse interfaces. On the right top is a filter field and a button with extra filter options to specify, for example, complete/incomplete/canceled, due, and priority.  Tags can be specified in the search field like any other word.

By default just show incomplete.  That should be a multi-select list though.

Under the search box is a list of tasks matching the filters.  Clicking a task should take you to the file and the place in a file where the task is so you can see the context.

When a folder is selected, it narrows the list of tasks accordingly to things in that folder and subfolders.

The list of tasks will, for now, have two "modes" which will be like centered tab icons between the search and the results. Default to category.

Each task should be in its own sort of card and displayed nicely with tags in pills, a colored dot indicating priority, a nicely displayed calendar with due info and ditto for done info.

In the markdown, tasks can be subtasks in a situation like this:

```markdown
* [ ] parent task
	* [ ] broken down subtask 1
	* [ ] broken down subtask 2
```

We may change this in the future, but for now we'll treat all of these as independent tasks in this interface.

I also use a syntax for when I move tasks between files, especially between daily notes.  We don't need full support for this syntax at this time, but for now I'd like to treat these as canceled.  If I've moved something to a daily note, it looks like this:

```markdown
* [>] This task was moved to a specific date > 2026-08-04
```

On the flip side, I have a notation showing where a task came from that puts something like `< 2026-08-01` on the end of the task.  We should probably detect that and omit it from the display for now.

Should be able to use ctrl-n/ctrl-p and arrow keys to navigate focused tasks and to use space to toggle done/ not done for selected item or press enter to jump to the task in a target doc. In many ways, this is like our search and should feel similar.

### Category Mode

The list of tasks will have headings for groups and the heading will be the title of the note the tasks were found in. Maybe the folder under that in small font.

Each heading should have right aligned on it a x/y indicator and small progress bar showing how many tasks in the file are done.  Clicking a heading should collapse the items under it. The x/y should include all tasks in the file even if they've been filtered out of the view.

### Calendar Mode

Instead of sorting by file and then ordering by order within the file, this will display tasks with due dates grouped with headings like: Overdue, Today, Tomorrow, and Upcoming (with smaller headings for each date grouping items under upcoming. This is all in a scrolling vertical list. [Taskview](https://github.com/Gimanh/taskview-community) has a nice UI for this (see [screenshot](https://github.com/Gimanh/taskview-community/blob/main/assets/taskview/main-dark.png)). 

Each heading should have right aligned on it a x/y indicator and small progress bar showing how many tasks are done, but only for today, tomorrow, and upcoming.  No progress for specific days in upcoming nor for overdue.  Clicking a heading should collapse the items under it. The x/y should include all tasks for the given due date, but filtered by any filters excluding the done/not done/cancel status.

When it comes to tasks that are canceled, just ignore them entirely.  They do not count towards totals.

## Editing

When edit mode is on, we should be able to click checkboxes to mark things as done/not done, or, on right-click, to cancel.  That's within the standard markdown viewing interface as well as the popup list of tasks.  Any other editing for now will require editing the file and dealing with the markdown there.

## Applicability

For now, lets hold the tasks stuff entirely out of static builds. This will be for server/gui mode only as that implies "live" files not snapshot in time stuff.
