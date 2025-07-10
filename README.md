---
title: something
tags:
    - x
    - y
    - z
---

# mbr - markdown browser

The goal of this is to preview markdown under an assumption that there are other markdown files around and we want to be able to jump around between them by following links, browsing tags, browsing folders, and searching. Ultimately things like backlinks, in-document table of contents, and more.

https://www.youtube.com/watch?v=3Q08a7BI9XI

That above is a test.

## TODO

* Web server
	* [ ] Establish the root directory
        * I want to automatically detect the "root" folder, which will look in CWD for a `.mbr/` folder, and will look in each parent dir for that
        * If found, that's our root.  If not found, the CWD is the root.
	* [ ] Establish the path to the markdown file relative to the root directory
	* [ ] Make sure the server is in that context and passed in URL is too
	* [ ] Serve .md files
	* [ ] Serve static files
		* In .mbr as well as in the static dir
		* I want to serve files looking first for markdown for a URL, then static inline with the markdown files, then finally the static folder fallback
		* How to handle index.md files?
	* ---
	* [ ] Serve ranged requests for videos (info: https://github.com/tokio-rs/axum/pull/3047 and https://github.com/tokio-rs/axum/blob/main/examples/static-file-server/src/main.rs)
	* [ ] tls? https://github.com/tokio-rs/axum/tree/main/examples/tls-rustls
	* [ ] Websockets route to push when active file is changed on disk
	* [ ] Not sure if I'll need hls but https://docs.rs/hls/0.5.5/hls/ is something I might look at

* Configs
	* [x] Listen IP
	* [x] Listen Port
	* [x] Config file in `.mbr/config.toml` or defaults
	* [x] Optional static directory
	* [ ] Gzip?
	* [x] Enable writes
	* [x] Markdown extension(s)
	* [x] theme.css

* Markdown parsing
	* [ ] Make all links relative so for example from `/xyz` to `../../xyz` as needed which will handle static generation hosted mode and prefixes and more
        * All links that are relative will need to be converted 
        * in arbitrary subfolders
        * Also, don't allow `..` paths that go outside of the root
	* [ ] Parse YAML frontmatter and make it available as JSON in the HTML doc
	* [ ] For image links, check the suffix for known audio and video and use different embeds for those
	* [ ] For youtube, use a youtube component (instead of oembed?  yeah, I think so)
	* [ ] For bare links on their own lines, use a component and pass as much info as possible
		* Bother with oembed at all?
	* [ ] Add code block special handling
		* Figure out code syntax highlighting -- client side? maybe a `<mbr-code language="[language]">` component.
		* Check for a special `.mbr/code/[language].js` file and if it exists, use a `<mbr-code-[language]>` component instead
		* Test feature by implementing a mermaid component -- and bake it in so it's a default (see https://github.com/glueball/simple-mermaid/blob/main/src/lib.rs)

* Templates and skinning 
	* [ ] Default "type" should be "note"; look for `.mbr/templates/note.html` and if it doesn't exist, fallback to a hardcoded default
	* [ ] Look for `.mbr/themes/theme.css` and fallback to hardcoded internal if it is missing (maybe lazy static read from a file?) -- and use config to determine theme
	* [ ] Look for `.mbr/themes/user.css` and fallback to hardcoded internal if it is missing (maybe lazy static read from a file?)
	* [ ] Update the default note.html to use some web components
		* Override web components with `.mbr/components/[comp].js` and read in and hard code defaults
        // TODO: mermaid 
