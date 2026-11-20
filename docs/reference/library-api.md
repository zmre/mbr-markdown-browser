---
title: Library API
description: "Using mbr's note index from Rust: mbr_core::Site — notes, frontmatter, backlinks, search"
order: 5
---

# Library API

An application written in Rust can use mbr's note index without running a
server: list the notes of a kind, read one note's whole frontmatter, find what
links to it, search. That is `mbr_core::Site`, a small façade over the same
scan and indexes the server and `mbr -b` use.

```toml
# Cargo.toml
[dependencies]
mbr-core = "0.7"
```

```rust
use std::path::Path;
use mbr_core::{Config, Site};

let root = Path::new("/path/to/notes");
let site = Site::open(root, Config::read(root)?)?;

for customer in site.notes_of_type("organization") {
    let note = site.note(&customer.path)?;          // frontmatter, contact, body
    let terms = note.frontmatter.get("terms");      // a number stays a number
    let invoices = site.backlinks(&customer.path);  // who links here
    println!("{}: {terms:?}, {} backlinks", customer.title, invoices.len());
}
```

A runnable version is `crates/mbr-core/examples/list_notes.rs`
(`cargo run -p mbr-core --example list_notes -- ~/notes organization terms`).
The rustdoc of `mbr_core::site` has every detail.

## Methods

| Method | Returns |
|--------|---------|
| `Site::open(root, config)` | Scans `root` (it replaces `config.root_dir`) and builds the wikilink, tag and relationship indexes. `Config::read(root)` applies the repository's `.mbr/config.toml` and `MBR_*` |
| `refresh(&[PathBuf])` | Applies changes to those files (repository-relative or absolute): re-read if they exist, dropped if not, every index updated incrementally |
| `notes()` | Every note as a `NoteSummary` — `path` (repository-relative), `url`, `title`, `note_type` (`type`), `tags` — in path order |
| `notes_of_type(ty)` | The notes whose `type` is `ty`, case-insensitive |
| `notes_with_fields(ty, fields)` | Notes of a type (or all), each with the named frontmatter fields; a dotted name reads a nested map (`billing_portal.url`) |
| `note(path)` | `NoteData`: the summary, the whole `frontmatter` as a `serde_json::Value`, the `contact` model, the resolved `frontmatter_links`, the `body` and the file's `content_hash` |
| `frontmatter(path)` | The whole frontmatter only, from the first bytes of the file |
| `field_values(key)` | Each distinct value of a field with how many notes use it, most used first: for autocomplete |
| `backlinks(path)` | The notes linking to it, [frontmatter links](../features/links/#frontmatter-links) included (`origin` says which key) |
| `resolve_wikilink(from, target)` | Where `[[target]]` written in `from` leads, resolved like a wikilink in the body |
| `search(&SearchQuery)` | The search `POST /.mbr/search` runs |
| `repo()` | The scanned `Repo`, for anything else |

**The frontmatter is lossless.** Nested maps stay maps, lists of maps stay
lists, numbers stay numbers — the same value per-note page templates get as
`frontmatter` (`mbr_core::markdown::lossless_frontmatter`, and
`mbr_core::markdown::yaml_to_json` for a single YAML value).

**Contact fields are not stripped.** `site.json` leaves out a contact's
`emails`, `phones`, `addresses`, … because a published site should. A library
caller owns the files, so `note`, `notes_with_fields` and `field_values`
include them. Over HTTP they need edit access; see below.

**Writing.** `Site` reads. To change a note, write the file and call
`refresh` with its path. `NoteData::content_hash` is the fingerprint
`POST /.mbr/edit` takes as `base_hash`, if you write through the server
instead.

## Costs and threads

- `Site::open` scans the repository the way the server does at startup:
  in parallel, frontmatter only.
- The backlink index is built on the first `backlinks` call, by parsing every
  note once, and is kept current by `refresh`.
- Every method blocks. In async code, call them from `spawn_blocking`.
  `Site` is `Send + Sync`, so share it behind an `Arc`.

## With a server

An application that also serves mbr's pages (merging
`mbr_server::server::Server`'s `router` into its own) should not scan twice:
`Server::site()` returns a `Site` over the server's own repository and
backlink index. The server's file watcher keeps them current, so do not call
`refresh` on that `Site` while a watcher runs.

The same index is available over HTTP to a browser front end:

- [`GET /.mbr/notes?type=…&fields=…`](http-api/#note-index)
- [`GET /.mbr/fields/{key}/values`](http-api/#note-index)
