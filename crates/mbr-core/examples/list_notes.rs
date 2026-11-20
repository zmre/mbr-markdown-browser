//! Lists the notes of one `type` with a few of their fields and backlinks,
//! through the library façade (`mbr_core::Site`).
//!
//! ```bash
//! cargo run -p mbr-core --example list_notes -- ~/notes organization terms,emails
//! ```

use std::path::PathBuf;

use mbr_core::{Config, Site};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or("usage: list_notes <root> <type> [fields]")?,
    );
    let note_type = args
        .next()
        .ok_or("usage: list_notes <root> <type> [fields]")?;
    let fields: Vec<String> = args
        .next()
        .map(|f| f.split(',').map(str::to_string).collect())
        .unwrap_or_default();

    // The repository's own `.mbr/config.toml` and `MBR_*` apply, as for `mbr -s`.
    let config = Config::read(&root)?;
    let site = Site::open(&root, config)?;

    for note in site.notes_with_fields(Some(&note_type), &fields) {
        let backlinks = site.backlinks(&note.summary.path);
        println!(
            "{} ({}) — {} backlink(s)",
            note.summary.title,
            note.summary.url,
            backlinks.len()
        );
        for (key, value) in &note.fields {
            println!("    {key}: {value}");
        }
    }
    Ok(())
}
