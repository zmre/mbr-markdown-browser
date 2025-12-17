---
title: something
keyword: supercalifragilistic
---

# mbr - markdown browser

The goal of this is to preview markdown under an assumption that there are other markdown files around and we want to be able to jump around between them by following links, browsing tags, browsing folders, and searching. Ultimately, things like backlinks, in-document table of contents, and more will be available.  So first it's a markdown previewer, but then a markdown browser for navigating markdown files. And finally, I want it to be an optional static site generator.

A key principle is that any given repo of markdown files can have the UI for browsing it customized -- not just styling but pretty much everything.  Users customize by creating a `.mbr/` folder in the root of their markdown file project.  Inside that, there can be a `config.toml` file for customizing.  Additionally, all javascript, css, and html components are fetched from a URL that is `/.mbr/something` and it will look in the local `.mbr/` folder for the files before falling back to compiled-in defaults.

Also, some of my notes have a lot of embedded videos in them so I want these to work out of the box without much effort or any ugly syntax.  I'm adding some extensions to standard github flavored markdown to enable these things.

## Technical approach

1. Markdown will convert to HTML on the fly
2. HTML will be served up from a local private web server
3. The UI is HTML+JavaScript+CSS with Lit web components

Performance is extremely important -- for launch of GUI and server, render of a markdown, build of a site, and for built sites, loading and rendering in a browser.

## Running

For now, you always need to specify a markdown file, even if starting in server mode.

* `mbr README.md` will process the markdown and print it to the terminal
* `mbr -s README.md` will start the web server and point you at <http://127.0.0.1:5200/README/>
* `mbr -g README.md` will launch a window and the web server with the window automatically showing the correct URL

https://www.youtube.com/watch?v=gz9BRl7DVSM

## Developing

There's the rust dev and the html/javascript dev.

For the rust side, try: `cargo watch -q -c -x 'run --release -- -s README.md'`

Then in another tab, you need to run vite.  But you need vite to connect to rust...

## TODO

* App
  * Need an app shell on Mac so we can have nice icons, menus, etc.

* Web server
	* [x] Establish the root directory
        * I want to automatically detect the "root" folder, which will look in CWD for a `.mbr/` folder, and will look in each parent dir for that
        * If found, that's our root.  If not found, the CWD is the root.
	* [x] Establish the path to the markdown file relative to the root directory
	* [x] Make sure the server is in that context and passed in URL is too
	* [x] Serve .md files
	* [x] Serve static files
		* In .mbr as well as in the static dir
		* I want to serve files looking first for markdown for a URL, then static inline with the markdown files, then finally the static folder fallback
		* How to handle index.md files?
	* [ ] **Serve sections (default index files)**
	* [ ] tls? [see the axum tls-rustls example](https://github.com/tokio-rs/axum/tree/main/examples/tls-rustls)
	* [ ] Websockets route to push when active file is changed on disk
  * [ ] Add a "multi-server" option to serve up multiple different note routes; might require an architecture change and definitely requires a path prefix concept
    * would let me host my personal notes as well as magic notes as well as whatever

* Videos
	* [ ] Serve captions, chapters, and posters automatically
	* [ ] Serve ranged requests for videos (info: <https://github.com/tokio-rs/axum/pull/3047> and <https://github.com/tokio-rs/axum/blob/main/examples/static-file-server/src/main.rs>)
		* [ ] Need to figure out if this is already happening -- how to test?
	* [ ] Not sure if I'll need hls but <https://docs.rs/hls/0.5.5/hls/> is something I might look at

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
  * [ ] Allow unordered bullets under ordered and vice versa
	* [ ] Make all links relative so for example from `/xyz` to `../../xyz` as needed which will handle static generation hosted mode and prefixes and more
        * All links that are relative will need to be converted 
        * in arbitrary subfolders
        * Also, don't allow `..` paths that go outside of the root
		* Hmmm, what about those `/.mbr/whatever` files?  I'd have to change those differently on every page -- how to do this in the template?  ie, not just from markdown so maybe **need to post process the output html**
	* [x] Parse YAML frontmatter and make it available as JSON in the HTML doc and also as template vars @done(2025-08-12 5:21 PM)
	* [ ] For image links, check the suffix for known audio and video and use different embeds for those
		* [ ] audio
		* [x] video @done(2025-08-12 5:22 PM)
		* [ ] pdf -- show the first page as image and click to open the whole pdf
	* [ ] For youtube, use a youtube component (instead of oembed?  yeah, I think so)
	* [ ] For bare links on their own lines, use a component and pass as much info as possible
		* Bother with oembed at all?
	* [ ] Add code block special handling
		* ~~Figure out code syntax highlighting -- client side? maybe a `<mbr-code language="[language]">` component.~~
		* ~~Check for a special `.mbr/code/[language].js` file and if it exists, use a `<mbr-code-[language]>` component instead~~
		* Test feature by implementing a mermaid component -- and bake it in so it's a default (see https://github.com/glueball/simple-mermaid/blob/main/src/lib.rs)
		* No!
			* I want this to be some sort of progressive enhancement deal
			* I want to use classes to indicate the code language, when available, and also `data-*` attributes
			* Any processing of the code will be 100% client-side
			* So should have no problem converting mermaid by simply including the mermaid code or by making a custom element that on load finds all `code.mermaid` blocks and converts to the custom component or reads in and replaces with an image with a url or whatever
			* I guess if I want server-side processing for certain code components, then I will need to special case them

* Templates and skinning 
	* [x] Look for `.mbr/themes/theme.css` and fallback to hardcoded internal if it is missing (maybe lazy static read from a file?) -- and use config to determine theme
	* [x] Look for `.mbr/themes/user.css` and fallback to hardcoded internal if it is missing (maybe lazy static read from a file?)
	* [-] Special handling for code blocks if web components exist for them, like `mbr-code-mermaid`
		* ~~Override web components with `.mbr/components/[comp].js` and read in and hard code defaults~~
	* [ ] **Navigation**
		* [ ] Add `link rel="next"` and `link rel="prev"` links in the header and provide next/prev vars to the template
		* [ ] Breadcrumb var?
		* [ ] Create `/.mbr/browse.json` endpoint (dynamically created right now, but obviously could be stored for a static build -- cache it?) with list of all markdown files, all folders, all media, etc., as URLs with other data like modified date, created date in there.
			* Separate endpoint to relate files to metadata?  Or just do it all in one shot so titles and tags and everything is delivered?
			* Rationalize static folder, turn markdown files into directories, etc.
			* Make sure dot-files and dot-folders like `.git` are ignored -- and maybe we have an ignore config, too? But don't use `.gitignore` because things not checked into git are not necessarily things that should be ignored in our browser
		* [ ] Figure out search
			* In dynamic only or static only modes, this would be easy, but how to rationalize?
			* When dynamic, we want to ripgrep
			* When static, we want a tokens file and some sort of client-side search library like lunr
			* What if we have a static file that takes parameters?  Like `/.mbr/search.json` which in static land is all tokens, but in dynamic land can have params like `/.mbr/search.json?q=x+y+z` where the file returned is just tokens from relevant files?  But then static will keep refetching the same big file with different params so that sucks.  
			* Much though I hate it, I think we need to add a param to every page saying if it is static or not and the search stuff needs to behave differently depending. Which might also mean different endpoints.
				* Or just make the client-side search really good?
		* [ ] Track links out and links in between files
* Misc
	* [ ] Auto handle address already in use error by incrementing the port if we're running in GUI mode
