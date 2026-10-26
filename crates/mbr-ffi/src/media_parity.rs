//! Media resolution parity: a QuickLook preview, the desktop server and the
//! iOS core must load the same file for every local reference a note makes.
//!
//! QuickLook once resolved assets with its own copy of the server's rules,
//! which drifted: it joined `static_folder` onto the root and demanded the
//! result stay inside it, so a SvelteKit site (`src/routes` as the root,
//! `static_folder = "../../static"`) previewed every `/images/...` broken while
//! the server showed them. Now both go through `path_resolver`, and this table
//! holds them to it.
//!
//! One table of references ([`CASES`], [`FRONTMATTER_CASES`]) over one fixture
//! per static-overlay [`Layout`], read three ways, each reporting the bytes a
//! reference loads (`None`: nothing):
//!
//! - **QuickLook** (Apple targets): the preview's `cid:` attachments, and the
//!   thumbnail's cover image for frontmatter.
//! - **Desktop server** (`mobile` feature, the only build of this crate with
//!   `mbr-server` in it): the page rendered by `Server::init`'s router, each
//!   URL resolved against the page URL the way a browser does, then fetched.
//! - **iOS core** (`mobile`): the same through `MbrApp::handle`, for the
//!   in-root layout only. A vault is the folder the user granted, and the
//!   embedded server indexes no external overlay (`../static` is outside the
//!   security-scoped grant), so that is by design rather than drift.
//!
//! Each fixture file holds its own name, so a body identifies the file it came
//! from.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where the static folder sits relative to the markdown root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// `static_folder = "static"`, inside the root.
    InRoot,
    /// `project/content` + `project/static`.
    OneUp,
    /// SvelteKit: `project/src/routes` + `project/static`, index notes named
    /// `+page.md` — the layout of the reported bug.
    TwoUp,
}

impl Layout {
    const ALL: [Self; 3] = [Self::InRoot, Self::OneUp, Self::TwoUp];

    /// The markdown root, relative to the project.
    fn root(self) -> &'static str {
        match self {
            Self::InRoot => "",
            Self::OneUp => "content",
            Self::TwoUp => "src/routes",
        }
    }

    fn static_folder(self) -> &'static str {
        match self {
            Self::InRoot => "static",
            Self::OneUp => "../static",
            Self::TwoUp => "../../static",
        }
    }

    fn index_file(self) -> &'static str {
        match self {
            Self::TwoUp => "+page.md",
            _ => "index.md",
        }
    }
}

/// Which folder a fixture file is in.
#[derive(Clone, Copy, Debug)]
enum Place {
    /// The markdown root.
    Root,
    /// The static folder (in or out of the root, per [`Layout`]).
    Static,
    /// The root's parent: where a `../` past the root lands, so every escape
    /// case aims at a file that exists. Never reachable by a URL.
    Outside,
}

/// Every asset in the fixture.
const FILES: &[(Place, &str)] = &[
    (Place::Root, "notes/img/a.png"),
    (Place::Root, "notes/Pasted image 1.png"),
    (Place::Root, "notes/srcset.png"),
    (Place::Root, "images/b.png"),
    (Place::Root, ".hidden/secret.png"),
    (Place::Static, "images/blog/hero.png"),
    (Place::Static, "images/blog/cover.jpg"),
    (Place::Static, "images/my photo.png"),
    (Place::Static, "pics/rel.png"),
    (Place::Outside, "outside.png"),
];

/// How a reference is written in the note.
#[derive(Clone, Copy, Debug)]
enum Markup {
    /// `![alt](url)`
    Markdown,
    /// `<img src="url" alt="alt">`, a raw HTML block.
    HtmlImg,
    /// `<img srcset="url 2x" alt="alt">`, a raw HTML block.
    HtmlSrcset,
}

/// One reference in the body of the note.
struct Case {
    /// Unique per case; how the reference is found in the rendered page.
    alt: &'static str,
    markup: Markup,
    /// As authored, from a note in `notes/`.
    url: &'static str,
    /// The file it must load, or `None` for nothing.
    expect: Option<(Place, &'static str)>,
}

const CASES: &[Case] = &[
    Case {
        alt: "case-note-relative",
        markup: Markup::Markdown,
        url: "img/a.png",
        expect: Some((Place::Root, "notes/img/a.png")),
    },
    Case {
        alt: "case-dot-relative",
        markup: Markup::Markdown,
        url: "./img/a.png",
        expect: Some((Place::Root, "notes/img/a.png")),
    },
    Case {
        alt: "case-parent-relative",
        markup: Markup::Markdown,
        url: "../images/b.png",
        expect: Some((Place::Root, "images/b.png")),
    },
    Case {
        alt: "case-root-relative",
        markup: Markup::Markdown,
        url: "/images/b.png",
        expect: Some((Place::Root, "images/b.png")),
    },
    Case {
        alt: "case-static-root-relative",
        markup: Markup::Markdown,
        url: "/images/blog/hero.png",
        expect: Some((Place::Static, "images/blog/hero.png")),
    },
    Case {
        alt: "case-static-relative",
        markup: Markup::Markdown,
        url: "../pics/rel.png",
        expect: Some((Place::Static, "pics/rel.png")),
    },
    Case {
        alt: "case-angle-spaces",
        markup: Markup::Markdown,
        url: "<Pasted image 1.png>",
        expect: Some((Place::Root, "notes/Pasted image 1.png")),
    },
    Case {
        alt: "case-encoded-spaces",
        markup: Markup::Markdown,
        url: "Pasted%20image%201.png",
        expect: Some((Place::Root, "notes/Pasted image 1.png")),
    },
    Case {
        alt: "case-static-encoded-spaces",
        markup: Markup::Markdown,
        url: "/images/my%20photo.png",
        expect: Some((Place::Static, "images/my photo.png")),
    },
    Case {
        alt: "case-html-relative",
        markup: Markup::HtmlImg,
        url: "img/a.png",
        expect: Some((Place::Root, "notes/img/a.png")),
    },
    Case {
        alt: "case-html-static",
        markup: Markup::HtmlImg,
        url: "/images/blog/hero.png",
        expect: Some((Place::Static, "images/blog/hero.png")),
    },
    Case {
        alt: "case-srcset-relative",
        markup: Markup::HtmlSrcset,
        url: "srcset.png",
        expect: Some((Place::Root, "notes/srcset.png")),
    },
    Case {
        alt: "case-srcset-static",
        markup: Markup::HtmlSrcset,
        url: "/images/blog/hero.png",
        expect: Some((Place::Static, "images/blog/hero.png")),
    },
    Case {
        alt: "case-missing-relative",
        markup: Markup::Markdown,
        url: "img/missing.png",
        expect: None,
    },
    Case {
        alt: "case-missing-root-relative",
        markup: Markup::Markdown,
        url: "/images/none.png",
        expect: None,
    },
    Case {
        alt: "case-escape-relative",
        markup: Markup::Markdown,
        url: "../../outside.png",
        expect: None,
    },
    Case {
        alt: "case-escape-root-relative",
        markup: Markup::HtmlImg,
        url: "/../outside.png",
        expect: None,
    },
    Case {
        alt: "case-escape-encoded",
        markup: Markup::HtmlImg,
        url: "/%2E%2E/outside.png",
        expect: None,
    },
    Case {
        alt: "case-hidden",
        markup: Markup::Markdown,
        url: "/.hidden/secret.png",
        expect: None,
    },
];

/// A frontmatter image, each in a note of its own (`notes/cover-N.md`): a
/// thumbnail uses the first usable one, so one per note keeps them apart.
struct FrontmatterCase {
    key: &'static str,
    value: &'static str,
    expect: Option<(Place, &'static str)>,
}

const FRONTMATTER_CASES: &[FrontmatterCase] = &[
    FrontmatterCase {
        key: "image",
        value: "/images/blog/cover.jpg",
        expect: Some((Place::Static, "images/blog/cover.jpg")),
    },
    FrontmatterCase {
        key: "cover",
        value: "/images/b.png",
        expect: Some((Place::Root, "images/b.png")),
    },
    FrontmatterCase {
        key: "cover",
        value: "img/a.png",
        expect: Some((Place::Root, "notes/img/a.png")),
    },
    FrontmatterCase {
        key: "image",
        value: "/images/my%20photo.png",
        expect: Some((Place::Static, "images/my photo.png")),
    },
    FrontmatterCase {
        key: "image",
        value: "/images/blog/missing.jpg",
        expect: None,
    },
    FrontmatterCase {
        key: "cover",
        value: "../../outside.png",
        expect: None,
    },
];

/// The note body holding every [`CASES`] reference.
fn note_body() -> String {
    CASES
        .iter()
        .map(|case| {
            let (alt, url) = (case.alt, case.url);
            match case.markup {
                Markup::Markdown => format!("![{alt}]({url})\n\n"),
                Markup::HtmlImg => format!("<img src=\"{url}\" alt=\"{alt}\">\n\n"),
                Markup::HtmlSrcset => format!("<img srcset=\"{url} 2x\" alt=\"{alt}\">\n\n"),
            }
        })
        .collect()
}

/// A project on disk laid out per [`Layout`], with every [`FILES`] asset, the
/// index and plain notes holding [`note_body`], and the cover notes.
struct Fixture {
    _tmp: tempfile::TempDir,
    /// Canonical, so expected paths compare equal to resolved ones.
    project: PathBuf,
    layout: Layout,
}

impl Fixture {
    fn new(layout: Layout) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).expect("create project");
        let project = project.canonicalize().expect("canonical project");
        let fixture = Self {
            _tmp: tmp,
            project,
            layout,
        };
        let config = format!(
            "static_folder = \"{}\"\nindex_file = \"{}\"\n",
            layout.static_folder(),
            layout.index_file()
        );
        write(&fixture.root().join(".mbr/config.toml"), config.as_bytes());
        for &(place, path) in FILES {
            write(&fixture.path(place, path), path.as_bytes());
        }
        for (note, _) in fixture.notes() {
            write(&fixture.root().join(note), note_body().as_bytes());
        }
        for (i, case) in FRONTMATTER_CASES.iter().enumerate() {
            let note = format!("---\n{}: \"{}\"\n---\n# Cover {i}\n", case.key, case.value);
            write(
                &fixture.root().join(format!("notes/cover-{i}.md")),
                note.as_bytes(),
            );
        }
        fixture
    }

    fn root(&self) -> PathBuf {
        self.project.join(self.layout.root())
    }

    fn path(&self, place: Place, path: &str) -> PathBuf {
        match place {
            Place::Root => self.root().join(path),
            Place::Static => self.project.join("static").join(path),
            Place::Outside => self.root().parent().expect("root has a parent").join(path),
        }
    }

    /// The notes holding [`CASES`] and their served URLs: an index note,
    /// served at its folder's URL, and a plain one at its own, whose relative
    /// links the renderer writes one `../` deeper.
    fn notes(&self) -> [(String, &'static str); 2] {
        [
            (format!("notes/{}", self.layout.index_file()), "/notes/"),
            ("notes/note.md".to_string(), "/notes/note/"),
        ]
    }

    /// What `expect` must load: the file's bytes.
    fn expected(&self, expect: Option<(Place, &str)>) -> Option<Vec<u8>> {
        expect.map(|(place, path)| std::fs::read(self.path(place, path)).expect("fixture file"))
    }

    /// Panics listing every case where `loaded` (alt or frontmatter label →
    /// bytes loaded) differs from what the table expects.
    fn assert_loads(
        &self,
        column: &str,
        note: &str,
        loaded: &BTreeMap<String, Option<Vec<u8>>>,
        expected: impl Iterator<Item = (String, Option<(Place, &'static str)>)>,
    ) {
        let show = |bytes: &Option<Vec<u8>>| {
            bytes.as_ref().map_or_else(
                || "nothing".to_string(),
                |bytes| format!("{:?}", String::from_utf8_lossy(bytes)),
            )
        };
        let failures: Vec<String> = expected
            .filter_map(|(label, expect)| {
                let want = self.expected(expect);
                let got = loaded.get(&label).cloned().flatten();
                (got != want)
                    .then(|| format!("  {label}: loaded {}, expected {}", show(&got), show(&want)))
            })
            .collect();
        assert!(
            failures.is_empty(),
            "{column}, {:?} layout, {note}:\n{}",
            self.layout,
            failures.join("\n")
        );
    }

    fn assert_cases(&self, column: &str, note: &str, loaded: &BTreeMap<String, Option<Vec<u8>>>) {
        let expected = CASES.iter().map(|case| (case.alt.to_string(), case.expect));
        self.assert_loads(column, note, loaded, expected);
    }

    fn assert_frontmatter(&self, column: &str, loaded: &BTreeMap<String, Option<Vec<u8>>>) {
        let expected = FRONTMATTER_CASES
            .iter()
            .enumerate()
            .map(|(i, case)| (frontmatter_label(i), case.expect));
        self.assert_loads(column, "notes/cover-N.md", loaded, expected);
    }
}

fn frontmatter_label(i: usize) -> String {
    let case = &FRONTMATTER_CASES[i];
    format!("cover-{i} ({}: {})", case.key, case.value)
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("create dir");
    std::fs::write(path, bytes).expect("write fixture file");
}

/// The (HTML-unescaped) value of attribute `name` on the tag carrying
/// `alt="{alt}"`, or `None` when there is no such tag or attribute.
fn attribute(html: &str, alt: &str, name: &str) -> Option<String> {
    let at = html.find(&format!(r#"alt="{alt}""#))?;
    let tag = &html[html[..at].rfind('<')?..at + html[at..].find('>')?];
    let marker = format!(r#" {name}=""#);
    let value = &tag[tag.find(&marker)? + marker.len()..];
    Some(html_escape::decode_html_entities(&value[..value.find('"')?]).into_owned())
}

/// The URL a reference's tag loads: its `src`, or the first `srcset`
/// candidate.
fn loaded_url(html: &str, case: &Case) -> Option<String> {
    match case.markup {
        Markup::Markdown | Markup::HtmlImg => attribute(html, case.alt, "src"),
        Markup::HtmlSrcset => attribute(html, case.alt, "srcset")?
            .split_whitespace()
            .next()
            .map(str::to_string),
    }
}

#[cfg(target_vendor = "apple")]
mod quicklook {
    use super::*;
    use crate::quicklook::{ThumbnailDocument, render_preview, render_thumbnail};

    /// What the preview of `note` loads for each case (the attachment behind
    /// its `cid:` URL), and every attachment the table did not ask for — the
    /// attachment list is everything a preview can read, so it must be empty.
    fn preview_loads(
        fixture: &Fixture,
        note: &str,
    ) -> (BTreeMap<String, Option<Vec<u8>>>, Vec<String>) {
        let path = fixture.root().join(note);
        let document = render_preview(path.to_string_lossy().into_owned(), None).expect("preview");
        let expected_files: Vec<PathBuf> = CASES
            .iter()
            .filter_map(|case| case.expect)
            .map(|(place, path)| fixture.path(place, path))
            .collect();
        let unexpected = document
            .attachments
            .iter()
            .filter(|a| !expected_files.contains(&PathBuf::from(&a.path)))
            .map(|a| a.path.clone())
            .collect();
        let loaded = CASES
            .iter()
            .map(|case| {
                let bytes = loaded_url(&document.html, case)
                    .and_then(|url| url.strip_prefix("cid:").map(str::to_string))
                    .and_then(|id| document.attachments.iter().find(|a| a.id == id))
                    .map(|attachment| std::fs::read(&attachment.path).expect("attachment"));
                (case.alt.to_string(), bytes)
            })
            .collect();
        (loaded, unexpected)
    }

    #[test]
    fn quicklook_preview_loads_what_the_table_expects() {
        for layout in Layout::ALL {
            let fixture = Fixture::new(layout);
            for (note, _) in fixture.notes() {
                let (loaded, unexpected) = preview_loads(&fixture, &note);
                fixture.assert_cases("QuickLook preview", &note, &loaded);
                assert!(
                    unexpected.is_empty(),
                    "{layout:?} layout, {note}: unexpected attachments {unexpected:?}"
                );
            }
        }
    }

    #[test]
    fn quicklook_thumbnail_cover_loads_what_the_table_expects() {
        for layout in Layout::ALL {
            let fixture = Fixture::new(layout);
            let loaded = (0..FRONTMATTER_CASES.len())
                .map(|i| {
                    let path = fixture.root().join(format!("notes/cover-{i}.md"));
                    let bytes = match render_thumbnail(path.to_string_lossy().into_owned(), None) {
                        Ok(ThumbnailDocument::Image { path }) => {
                            Some(std::fs::read(path).expect("cover"))
                        }
                        Ok(ThumbnailDocument::Page { .. }) => None,
                        Err(e) => panic!("thumbnail of cover-{i}: {e}"),
                    };
                    (frontmatter_label(i), bytes)
                })
                .collect();
            fixture.assert_frontmatter("QuickLook thumbnail", &loaded);
        }
    }
}

#[cfg(feature = "mobile")]
mod served {
    use super::*;
    use crate::mobile::{MOBILE_HOST, MbrApp, transport};
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Something answering `GET` for a URL path: status and body.
    trait Fetch {
        fn get(&self, path: &str) -> (u16, Vec<u8>);
    }

    /// `relative` resolved against `page_url` the way a browser resolves a
    /// URL in a page: a `/`-led one as is, any other against the page URL's
    /// folder, `.` dropped and `..` clamped at the root.
    fn resolve_url(page_url: &str, relative: &str) -> String {
        let joined = if relative.starts_with('/') {
            relative.to_string()
        } else {
            let folder = &page_url[..=page_url.rfind('/').unwrap_or(0)];
            format!("{folder}{relative}")
        };
        let mut segments: Vec<&str> = Vec::new();
        for segment in joined.split('/').skip(1) {
            match segment {
                "." => {}
                ".." => {
                    segments.pop();
                }
                _ => segments.push(segment),
            }
        }
        format!("/{}", segments.join("/"))
    }

    /// What `GET path` loads: its body when the answer is `200`.
    fn load(fetch: &dyn Fetch, path: &str) -> Option<Vec<u8>> {
        let (status, body) = fetch.get(path);
        (status == 200).then_some(body)
    }

    /// What each case loads from the page at `page_url`.
    fn page_loads(fetch: &dyn Fetch, page_url: &str) -> BTreeMap<String, Option<Vec<u8>>> {
        let (status, page) = fetch.get(page_url);
        assert_eq!(status, 200, "GET {page_url}");
        let html = String::from_utf8(page).expect("UTF-8 page");
        CASES
            .iter()
            .map(|case| {
                let bytes = loaded_url(&html, case)
                    .and_then(|url| load(fetch, &resolve_url(page_url, &url)));
                (case.alt.to_string(), bytes)
            })
            .collect()
    }

    /// What each frontmatter value loads, read as a URL relative to the
    /// cover note's folder (`/notes/`), as QuickLook reads it.
    fn frontmatter_loads(fetch: &dyn Fetch) -> BTreeMap<String, Option<Vec<u8>>> {
        FRONTMATTER_CASES
            .iter()
            .enumerate()
            .map(|(i, case)| {
                (
                    frontmatter_label(i),
                    load(fetch, &resolve_url("/notes/", case.value)),
                )
            })
            .collect()
    }

    fn assert_column(column: &str, fixture: &Fixture, fetch: &dyn Fetch) {
        for (note, page_url) in fixture.notes() {
            fixture.assert_cases(column, &note, &page_loads(fetch, page_url));
        }
        fixture.assert_frontmatter(column, &frontmatter_loads(fetch));
    }

    /// The desktop server, as `mbr -s` builds it for the fixture's root.
    struct DesktopServer {
        router: axum::Router,
        _server: mbr_server::Server,
        runtime: tokio::runtime::Runtime,
    }

    impl DesktopServer {
        fn new(root: &Path) -> Self {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("runtime");
            let mut config = mbr_core::config::Config::read(root).expect("config");
            // No network in a test.
            config.oembed_timeout_ms = 0;
            let server = {
                let _entered = runtime.enter();
                mbr_server::Server::init(mbr_server::ServerConfig::from(&config)).expect("server")
            };
            Self {
                router: server.router.clone(),
                _server: server,
                runtime,
            }
        }
    }

    impl Fetch for DesktopServer {
        fn get(&self, path: &str) -> (u16, Vec<u8>) {
            self.runtime.block_on(async {
                let url = format!("mbr://{MOBILE_HOST}{path}");
                let request =
                    transport::build_request("GET", &url, &[], Vec::new()).expect("request");
                let response = self.router.clone().oneshot(request).await.expect("answer");
                let status = response.status().as_u16();
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("body");
                (status, body.to_vec())
            })
        }
    }

    impl Fetch for Arc<MbrApp> {
        fn get(&self, path: &str) -> (u16, Vec<u8>) {
            futures::executor::block_on(async {
                let response = self
                    .handle(
                        "GET".to_string(),
                        format!("mbr://{MOBILE_HOST}{path}"),
                        Vec::new(),
                        Vec::new(),
                    )
                    .await
                    .expect("handle");
                (response.status(), response.read_all().await.expect("body"))
            })
        }
    }

    #[test]
    fn desktop_server_loads_what_the_table_expects() {
        for layout in Layout::ALL {
            let fixture = Fixture::new(layout);
            let server = DesktopServer::new(&fixture.root());
            assert_column("desktop server", &fixture, &server);
        }
    }

    #[test]
    fn ios_core_loads_what_the_table_expects() {
        // In-root only: see the module docs.
        let fixture = Fixture::new(Layout::InRoot);
        let app = MbrApp::open(
            fixture.root().to_string_lossy().into_owned(),
            None,
            None,
            None,
        )
        .expect("open vault");
        assert_column("iOS core", &fixture, &app);
    }

    #[test]
    fn resolve_url_resolves_like_a_browser() {
        assert_eq!(
            resolve_url("/notes/note/", "../img/a.png"),
            "/notes/img/a.png"
        );
        assert_eq!(resolve_url("/notes/", "img/a.png"), "/notes/img/a.png");
        assert_eq!(resolve_url("/notes/", "./img/./a.png"), "/notes/img/a.png");
        assert_eq!(resolve_url("/notes/", "../../../x.png"), "/x.png");
        assert_eq!(resolve_url("/notes/", "/images/b.png"), "/images/b.png");
        assert_eq!(resolve_url("/notes/", "/../x.png"), "/x.png");
    }
}
