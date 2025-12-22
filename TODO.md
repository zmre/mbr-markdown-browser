# TODO

## What's Next

1. [x] Investigate adding a menubar on mac and allowing for shortcut keys like cmd-q and cmd-w to work.  If all that works okay, see about setting up an About screen and application icon.  We want to be able to launch our markdown viewer both from the commandline (eg, `mbr /path/to/repo/or/md/file`) or via GUI.  But this is cross platform so we want it to work in expected ways on mac, linux, and windows using compile time flags to manage what code is needed for the application wrapper on each platform.  If there are multiple possible approaches to this, please research deeply and present pro's and con's and recommendations before moving forward.
2. [x] Refactor the html template files a bit so we don't have duplicated code making use of includes for common portions (header, footer, etc.) via tera. The shared files don't need to be exposed as URLs.
3. [x] Right now, if we load a markdown file, in the html head we have json with details about the underlying markdown in json, which can be used by components on the page. But the raw markdown is included in that json and we try to then delete it after the fact on page load to free memory, but this is inefficient. We should filter out the markdown before adding the file details to the header.
4. [x] Site build functionality: in addition to being a live markdown previewer and browser, this tool can be used as a static site builder. Generates HTML for all markdown files and directories, symlinks assets (macOS/Linux only), copies `.mbr/` folder with defaults, and creates `site.json`. Use `-b/--build` flag with optional `--output` for custom directory. @done(2025-12-20)
5. [x] Search functionality: this app is not just a markdown previewer, but a repository browser. We want to be able to link between files, browse, and search the repo.  Search is the first time we will have a divergence in build behavior versus live server or gui behavior. We also need to give the frontend a way to discern whether there's a dynamic server available (probably as part of the JSON in the <head>) or not so the frontend search widget can adjust as needed.  We want faceted search with prioritized fields basically amounting to: markdown filename/path and title are highest priority. Next highest are the frontmatter fields (tags or category or date or anything else that is there). Finally the contents. As a nice to have, headers inside markdown should be considered more important than other text, but as runtime performance is critical, this may not be possible.  Additionally, we want to be able to search just markdown files and, optionally, ALL files, especially including searchable PDFs, VTT files with captions or chapters, and paths of other files like images. Filetype therefore should be another search facet. Never shell out to other tools. Despite there being two different ways to search depending on mode, search syntax and user experience must be the same. Another facet would be to search just under the current folder (for the file being viewed) or the whole repo.
  a. **Live Server Behavior**: For searching titles, for example, or pathnames, could simply use the preloaded (possibly -- or when it finishes loading) index of repo files and metadata. That may be possible for other frontmatter like tags, too. For body content, use the [ripgrep](https://crates.io/crates/ripgrep) crate to search through specified files / file types. There will be a POST endpoint for `/search?q=` which returns json for ordered results (with some limit, default 50) that returns URL path, title, and other frontmatter info for each file and, if available, a snippet excerpt.  Note: there may be a crate that handles some of these concerns well, but lets research it for how active the repo is, how well maintained, how broadly used, how long issues linger, and so forth before making decisions on build vs. using a crate.
  b. **Built Site Behavior**: In this case, search will be local. In fact, we can make parts of search local with live server behavior, too, to keep the experiences as similar as possible.  For full content search where the server will use ripgrep in the repo, the built site will make use of a client-side index that is created at site build time using [Microsoft Docfind](https://github.com/microsoft/docfind/tree/main).
6. [x] We have a minor issue where when we generate the static build (also in gui and server mode), ffmpeg sends a bunch of warnings to the console (probably to stderr, but that should be confirmed). this likely happens while fetching metadata from videos.  i've been testing in the ~/Documents/Magic directory which produces lots of these.  use that if needed. i don't want any ffmpeg output to show up.
7. [ ] Inside a markdown file, links are relative to the file. So a link that has no leading path part (no ../ or /) is in the same folder. But when the markdown file is turned into a url path part, then to reference that other file as a url, we need a leading `../`. When rendering markdown, some relative links need to be adjusted so as not to 404. We're going to need extensive tests on this to make sure we don't create inadvertant 404s
8. [ ] We need to level up our search in a couple of ways. First, we need to recognize that different repositories of markdown have different conventions and different frontmatter in the markdown. We need to be able to search on arbitrary fields. And to unify the static and dynamic modes and the UI, we should be able to search specific tag fields (facet) using a `key:value` reference. We also need an option to search current folder and subfolders (and current folder should really be the parent of the current markdown file) or everywhere.  We need a second option to specify markdown files or all files. And we need to index other text files and pdfs in a separate index.
9. [ ] Get serious about the UI complete with vim-like shortcut keys (j/k, space, H/L, /, maybe ctrl-p, ctrl-f/b, ctrl-d/u, -, etc.)
10. [ ] Switch up so this is mostly a library and the cli just calls into the public library interface in different ways
11. [ ] On MacOS, when open mbr as an application, it sets the current working directory to the root, `/` and then proceeds to try to crawl over every file on the system. That isn't ideal. We need to handle when it is opened with a file or folder and if the app is opened with neither of these and in the root, it needs to launch a file open dialog for something to be selected, and then startup after a choice is made. there should also be a Open menu under File in the bar. If something is displayed and there's an Open and it goes to a new repo, we essentially have to start over on initialization for repo root, repo details, and everything else.
12. [ ] quicklook https://developer.apple.com/documentation/QuickLook

## Full List
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
	* [x] **Serve sections (default index files)** @done(2025-12-18 9:10 PM)
	* [ ] tls? [see the axum tls-rustls example](https://github.com/tokio-rs/axum/tree/main/examples/tls-rustls)
	* [x] Websockets route to push when active file is changed on disk @done(2025-12-18 10:39 PM)
  * [ ] Add a "multi-server" option to serve up multiple different note routes; might require an architecture change and definitely requires a path prefix concept
    * would let me host my personal notes as well as magic notes as well as whatever

* Videos
	* [ ] Serve captions, chapters, and posters automatically
	* [ ] Serve ranged requests for videos (info: <https://github.com/tokio-rs/axum/pull/3047> and <https://github.com/tokio-rs/axum/blob/main/examples/static-file-server/src/main.rs>)
		* [ ] Need to figure out if this is already happening -- how to test?
	* [-] Not sure if I'll need hls but <https://docs.rs/hls/0.5.5/hls/> is something I might look at

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
	* [x] For image links, check the suffix for known audio and video and use different embeds for those
		* [x] audio @done(2025-12-19 6:35 PM)
		* [x] video @done(2025-08-12 5:22 PM)
		* [x] pdf -- show the first page as image and click to open the whole pdf @done(2025-12-19 6:35 PM)
	* [x] For youtube, use a youtube component (instead of oembed?  yeah, I think so) @done(2025-12-19 6:36 PM)
	* [x] For bare links on their own lines, use a component and pass as much info as possible @done(2025-12-19 6:36 PM)
		* Bother with oembed at all?
	* [x] Add code block special handling
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
	* [x] Auto handle address already in use error by incrementing the port if we're running in GUI mode @done(2025-12-19)
  * [ ] Make a quicklook plugin that shows this!  That would be epic. Might need to inline all the dependencies?

