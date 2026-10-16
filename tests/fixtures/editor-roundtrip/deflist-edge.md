# Definition list edge cases

Where pulldown-cmark's `ENABLE_DEFINITION_LIST` starts, continues and ends a
definition. Each case is fenced off by a rule so it cannot join the list above.

Term
: definition straight after the term

***

Loose term

: definition after a blank line

: second definition after a blank line

***

Far term



: definition after several blank lines

***

First term
: first definition

Second term after a blank line continues the list
: its definition

***

Lazy term
: definition
lazy continuation of the definition
: second definition of the same term

***

A. one
B. two
C. three
D. four
: one multi-line term

***

Spaces
:no space after the colon
:  two spaces
:     five spaces: indented code

***

Empty
:
 one-space content after an empty marker

***

Outer
: outer definition
  Inner term
  : inner definition

***

Blocks
: paragraph

  second paragraph, loose

***

# Heading
: after a heading this is a paragraph

***

- list item

: after a list this is a paragraph

***

- item
: a lazy continuation of the item

***

- item
  Term in an item
  : definition in an item

***

- item
- : a new item's first line is a paragraph

***

> Quote term
> : quote definition

***

> quoted paragraph
: a lazy continuation of the quote

***

| a | b |
| - | - |
| 1 | 2 |
: a table row, not a definition

***

Definitions keep their content
: > a quote
: - a list
  - of two
: ```text
  a fence
  ```
: | x | y |
  | - | - |
  | 1 | 2 |
