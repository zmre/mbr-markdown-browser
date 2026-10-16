---
type: flashcard
---
# Rust ownership

What does `move` do in a closure?

: It moves every captured variable into the closure.

: ```rust
  let s = String::new();
  let f = move || s.len();
  ```

: ___Review History___

  * 2026-10-01 09:15 - Hard
  * 2026-10-04 18:02 - Good

Second card?
: First answer
: Second answer with a list:
  1. one
  2. two
