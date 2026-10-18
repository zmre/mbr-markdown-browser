# Empty definitions

An empty definition followed by a blank line ends the definition list
(pulldown-cmark pops the empty item, then the list): the next card starts a
list of its own instead of continuing this one.

First question?
: First answer.
: More about the first answer.
: 

Second question?\
\
A. Option one\
B. Option two
: **B.** Option two
:

Third question?
: Third answer.

Fourth question?
:

: Not a definition: the list above ended at the empty one, so this is a paragraph.

Fifth question?
: 
: A definition right after an empty one (no blank line) keeps the list going.

Sixth question?
: Sixth answer.

- A list item holding cards

  Nested question?
  : 

  Next nested question?
  : Nested answer.

- Another list item

  Nested question?
  :

  : Not a definition either: the nested list ended above.
