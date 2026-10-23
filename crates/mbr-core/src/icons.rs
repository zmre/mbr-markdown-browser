//! The icon set shared by templates and web components.
//!
//! The source of truth is `templates/icons/*.svg`: the inner shapes (no
//! `<svg>` wrapper) of vendored Lucide icons (ISC; notices in
//! `templates/icons/LICENSE`) plus mbr's own `footnote`. The components'
//! `components/src/icon-shapes.ts` is generated from the same files, so a
//! template and a component cannot draw different artwork for one name.
//!
//! [`icon_svg`] applies the one wrapper every icon uses: 24×24 viewBox, no
//! fill, `currentColor` stroke with round caps and joins, and the stroke width
//! from the `--mbr-icon-stroke` CSS token — never an attribute, so one token
//! sets the weight everywhere. Icons are decorative (`aria-hidden`); the
//! control around one carries its accessible name.
//!
//! Templates call it as the Tera function `icon(name="chevron-left", size=16)`
//! (registered by `templates.rs`); an unknown name is a render error, never
//! an empty string.

use std::collections::HashMap;

/// Every icon, `(name, shapes)`, sorted by name (pinned by a test, which also
/// checks the table lists exactly the files in `templates/icons/`).
pub const ICONS: &[(&str, &str)] = &[
    (
        "arrow-left",
        include_str!("../templates/icons/arrow-left.svg"),
    ),
    (
        "arrow-left-right",
        include_str!("../templates/icons/arrow-left-right.svg"),
    ),
    (
        "arrow-right",
        include_str!("../templates/icons/arrow-right.svg"),
    ),
    ("arrow-up", include_str!("../templates/icons/arrow-up.svg")),
    (
        "arrow-up-right",
        include_str!("../templates/icons/arrow-up-right.svg"),
    ),
    ("braces", include_str!("../templates/icons/braces.svg")),
    (
        "building-2",
        include_str!("../templates/icons/building-2.svg"),
    ),
    ("calendar", include_str!("../templates/icons/calendar.svg")),
    (
        "calendar-clock",
        include_str!("../templates/icons/calendar-clock.svg"),
    ),
    (
        "calendar-days",
        include_str!("../templates/icons/calendar-days.svg"),
    ),
    (
        "case-sensitive",
        include_str!("../templates/icons/case-sensitive.svg"),
    ),
    ("check", include_str!("../templates/icons/check.svg")),
    (
        "chevron-down",
        include_str!("../templates/icons/chevron-down.svg"),
    ),
    (
        "chevron-left",
        include_str!("../templates/icons/chevron-left.svg"),
    ),
    (
        "chevron-right",
        include_str!("../templates/icons/chevron-right.svg"),
    ),
    (
        "chevron-up",
        include_str!("../templates/icons/chevron-up.svg"),
    ),
    (
        "circle-help",
        include_str!("../templates/icons/circle-help.svg"),
    ),
    ("code", include_str!("../templates/icons/code.svg")),
    ("ellipsis", include_str!("../templates/icons/ellipsis.svg")),
    ("expand", include_str!("../templates/icons/expand.svg")),
    (
        "file-text",
        include_str!("../templates/icons/file-text.svg"),
    ),
    ("film", include_str!("../templates/icons/film.svg")),
    ("folder", include_str!("../templates/icons/folder.svg")),
    (
        "folder-input",
        include_str!("../templates/icons/folder-input.svg"),
    ),
    (
        "folder-open",
        include_str!("../templates/icons/folder-open.svg"),
    ),
    (
        "folder-tree",
        include_str!("../templates/icons/folder-tree.svg"),
    ),
    ("footnote", include_str!("../templates/icons/footnote.svg")),
    ("funnel", include_str!("../templates/icons/funnel.svg")),
    (
        "gallery-vertical-end",
        include_str!("../templates/icons/gallery-vertical-end.svg"),
    ),
    ("hash", include_str!("../templates/icons/hash.svg")),
    ("house", include_str!("../templates/icons/house.svg")),
    ("image", include_str!("../templates/icons/image.svg")),
    ("info", include_str!("../templates/icons/info.svg")),
    (
        "lightbulb",
        include_str!("../templates/icons/lightbulb.svg"),
    ),
    ("link", include_str!("../templates/icons/link.svg")),
    ("list", include_str!("../templates/icons/list.svg")),
    (
        "list-checks",
        include_str!("../templates/icons/list-checks.svg"),
    ),
    (
        "maximize-2",
        include_str!("../templates/icons/maximize-2.svg"),
    ),
    (
        "message-square-text",
        include_str!("../templates/icons/message-square-text.svg"),
    ),
    (
        "message-square-warning",
        include_str!("../templates/icons/message-square-warning.svg"),
    ),
    ("minus", include_str!("../templates/icons/minus.svg")),
    ("music", include_str!("../templates/icons/music.svg")),
    (
        "octagon-alert",
        include_str!("../templates/icons/octagon-alert.svg"),
    ),
    (
        "panel-left",
        include_str!("../templates/icons/panel-left.svg"),
    ),
    ("pencil", include_str!("../templates/icons/pencil.svg")),
    (
        "pencil-line",
        include_str!("../templates/icons/pencil-line.svg"),
    ),
    ("play", include_str!("../templates/icons/play.svg")),
    ("plus", include_str!("../templates/icons/plus.svg")),
    ("scan", include_str!("../templates/icons/scan.svg")),
    ("search", include_str!("../templates/icons/search.svg")),
    ("shrink", include_str!("../templates/icons/shrink.svg")),
    (
        "sliders-horizontal",
        include_str!("../templates/icons/sliders-horizontal.svg"),
    ),
    (
        "square-pen",
        include_str!("../templates/icons/square-pen.svg"),
    ),
    ("star", include_str!("../templates/icons/star.svg")),
    ("tag", include_str!("../templates/icons/tag.svg")),
    ("text", include_str!("../templates/icons/text.svg")),
    (
        "text-cursor-input",
        include_str!("../templates/icons/text-cursor-input.svg"),
    ),
    (
        "toggle-left",
        include_str!("../templates/icons/toggle-left.svg"),
    ),
    ("trash-2", include_str!("../templates/icons/trash-2.svg")),
    (
        "triangle-alert",
        include_str!("../templates/icons/triangle-alert.svg"),
    ),
    ("user", include_str!("../templates/icons/user.svg")),
    ("x", include_str!("../templates/icons/x.svg")),
];

/// Default size in CSS px: `md`, the header and panel size.
pub const DEFAULT_ICON_SIZE: u32 = 20;

/// The inner shapes of icon `name`, or `None` for an unknown name.
pub fn icon_shapes(name: &str) -> Option<&'static str> {
    ICONS
        .binary_search_by(|(candidate, _)| candidate.cmp(&name))
        .ok()
        .map(|index| ICONS[index].1.trim())
}

/// Icon `name` as a complete inline `<svg class="mbr-icon">`, `size` CSS px
/// square, or `None` for an unknown name.
pub fn icon_svg(name: &str, size: u32) -> Option<String> {
    icon_shapes(name).map(|shapes| {
        format!(
            "<svg class=\"mbr-icon\" viewBox=\"0 0 24 24\" width=\"{size}\" height=\"{size}\"              fill=\"none\" stroke=\"currentColor\" stroke-linecap=\"round\"              stroke-linejoin=\"round\" style=\"stroke-width:var(--mbr-icon-stroke,1.75)\"              aria-hidden=\"true\" focusable=\"false\">{shapes}</svg>"
        )
    })
}

/// Tera function `icon(name=…, size=…)`: the [`icon_svg`] markup, unescaped
/// by the caller with `| safe`. `size` defaults to [`DEFAULT_ICON_SIZE`].
pub fn icon_function(args: &HashMap<String, tera::Value>) -> tera::Result<tera::Value> {
    let name = args
        .get("name")
        .and_then(tera::Value::as_str)
        .ok_or_else(|| tera::Error::msg("icon(): `name` (a string) is required"))?;
    let size = match args.get("size") {
        None => DEFAULT_ICON_SIZE,
        Some(value) => value
            .as_u64()
            .and_then(|size| u32::try_from(size).ok())
            .filter(|size| (1..=512).contains(size))
            .ok_or_else(|| {
                tera::Error::msg("icon(): `size` must be a whole number from 1 to 512")
            })?,
    };
    icon_svg(name, size)
        .map(tera::Value::String)
        .ok_or_else(|| tera::Error::msg(format!("icon(): unknown icon `{name}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn table_is_sorted_and_unique() {
        assert!(
            ICONS.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "ICONS must be strictly sorted by name for binary_search"
        );
    }

    #[test]
    fn table_lists_exactly_the_icon_files() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/icons");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .expect("templates/icons exists")
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                name.strip_suffix(".svg").map(str::to_owned)
            })
            .collect();
        on_disk.sort();
        let in_table: Vec<&str> = ICONS.iter().map(|(name, _)| *name).collect();
        assert_eq!(in_table, on_disk, "regenerate ICONS from templates/icons/");
    }

    #[test]
    fn every_icon_is_shapes_only() {
        for (name, shapes) in ICONS {
            let shapes = shapes.trim();
            assert!(shapes.starts_with('<') && shapes.ends_with("/>"), "{name}");
            assert!(!shapes.contains("<svg"), "{name} must not carry a wrapper");
            assert!(!shapes.contains('\n'), "{name} must be one line");
        }
    }

    #[test]
    fn known_icon_renders_the_shared_wrapper() {
        let svg = icon_svg("x", 16).expect("x exists");
        assert!(svg.starts_with(
            "<svg class=\"mbr-icon\" viewBox=\"0 0 24 24\" width=\"16\" height=\"16\""
        ));
        assert!(svg.contains("stroke=\"currentColor\""));
        assert!(svg.contains("aria-hidden=\"true\""));
        assert!(svg.contains("<path d=\"M18 6 6 18\"/>"));
        assert!(svg.ends_with("</svg>"));
        assert!(
            !svg.contains("stroke-width=\""),
            "the weight comes from CSS"
        );
    }

    #[test]
    fn unknown_icon_is_none() {
        assert_eq!(icon_shapes("no-such-icon"), None);
        assert_eq!(icon_svg("no-such-icon", 20), None);
    }

    fn render(template: &str) -> tera::Result<String> {
        let mut tera = tera::Tera::default();
        tera.register_function("icon", icon_function);
        tera.add_raw_template("t", template)?;
        tera.render("t", &tera::Context::new())
    }

    #[test]
    fn tera_function_renders_known_icon() {
        let out = render("{{ icon(name=\"chevron-left\", size=16) | safe }}").unwrap();
        assert!(out.starts_with("<svg class=\"mbr-icon\""));
        assert!(out.contains("width=\"16\""));
        assert!(out.contains(icon_shapes("chevron-left").unwrap()));
    }

    #[test]
    fn tera_function_defaults_size() {
        let out = render("{{ icon(name=\"info\") | safe }}").unwrap();
        assert!(out.contains("width=\"20\" height=\"20\""));
    }

    #[test]
    fn tera_function_errors_on_unknown_name() {
        let err = render("{{ icon(name=\"nope\") | safe }}").unwrap_err();
        let chain = format!("{err:?}");
        assert!(chain.contains("unknown icon"), "{chain}");
    }

    #[test]
    fn tera_function_errors_on_bad_arguments() {
        assert!(render("{{ icon() | safe }}").is_err());
        assert!(render("{{ icon(name=\"x\", size=0) | safe }}").is_err());
        assert!(render("{{ icon(name=\"x\", size=\"big\") | safe }}").is_err());
    }
}
