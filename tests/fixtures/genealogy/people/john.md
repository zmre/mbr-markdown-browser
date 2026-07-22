---
type: person
title: John Doe
born: 1925-06-02
died: 1999-11-20
gender: male
relationships:
  # Reciprocal of George's "child -> John" (deduped to one edge).
  - type: parent
    to: "[[George Doe]]"
  - type: parent
    to: "[[Martha Doe]]"
  - type: spouse
    # referenced by married-name alias; resolves via Mary's `aliases`
    to: "[[Mary Doe]]"
    married: 1948-06-01
    place: "Denver, CO"
  - type: sibling
    to: "[[Robert Doe]]"
  - type: child
    to: "[[Alice Doe]]"
  - type: child
    to: "[[Sam Doe]]"
---

# John Doe

Second-generation Doe.
