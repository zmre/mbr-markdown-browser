# TODO

## BUGS

* browse is showing duplicates and then not showing children
* sections don't seem to be working right

## What's Next

1. [x] Investigate adding a menubar on mac and allowing for shortcut keys like cmd-q and cmd-w to work.  If all that works okay, see about setting up an About screen and application icon.  We want to be able to launch our markdown viewer both from the commandline (eg, `mbr /path/to/repo/or/md/file`) or via GUI.  But this is cross platform so we want it to work in expected ways on mac, linux, and windows using compile time flags to manage what code is needed for the application wrapper on each platform.  If there are multiple possible approaches to this, please research deeply and present pro's and con's and recommendations before moving forward.
2. [x] Refactor the html template files a bit so we don't have duplicated code making use of includes for common portions (header, footer, etc.) via tera. The shared files don't need to be exposed as URLs.
3. [x] Right now, if we load a markdown file, in the html head we have json with details about the underlying markdown in json, which can be used by components on the page. But the raw markdown is included in that json and we try to then delete it after the fact on page load to free memory, but this is inefficient. We should filter out the markdown before adding the file details to the header.
4. [x] Site build functionality: in addition to being a live markdown previewer and browser, this tool can be used as a static site builder. Generates HTML for all markdown files and directories, symlinks assets (macOS/Linux only), copies `.mbr/` folder with defaults, and creates `site.json`. Use `-b/--build` flag with optional `--output` for custom directory. @done(2025-12-20)
5. [x] Search functionality: this app is not just a markdown previewer, but a repository browser. We want to be able to link between files, browse, and search the repo.  Search is the first time we will have a divergence in build behavior versus live server or gui behavior. We also need to give the frontend a way to discern whether there's a dynamic server available (probably as part of the JSON in the <head>) or not so the frontend search widget can adjust as needed.  We want faceted search with prioritized fields basically amounting to: markdown filename/path and title are highest priority. Next highest are the frontmatter fields (tags or category or date or anything else that is there). Finally the contents. As a nice to have, headers inside markdown should be considered more important than other text, but as runtime performance is critical, this may not be possible.  Additionally, we want to be able to search just markdown files and, optionally, ALL files, especially including searchable PDFs, VTT files with captions or chapters, and paths of other files like images. Filetype therefore should be another search facet. Never shell out to other tools. Despite there being two different ways to search depending on mode, search syntax and user experience must be the same. Another facet would be to search just under the current folder (for the file being viewed) or the whole repo.
  a. **Live Server Behavior**: For searching titles, for example, or pathnames, could simply use the preloaded (possibly -- or when it finishes loading) index of repo files and metadata. That may be possible for other frontmatter like tags, too. For body content, use the [ripgrep](https://crates.io/crates/ripgrep) crate to search through specified files / file types. There will be a POST endpoint for `/search?q=` which returns json for ordered results (with some limit, default 50) that returns URL path, title, and other frontmatter info for each file and, if available, a snippet excerpt.  Note: there may be a crate that handles some of these concerns well, but lets research it for how active the repo is, how well maintained, how broadly used, how long issues linger, and so forth before making decisions on build vs. using a crate.
  b. **Built Site Behavior**: In this case, search will be local. In fact, we can make parts of search local with live server behavior, too, to keep the experiences as similar as possible.  For full content search where the server will use ripgrep in the repo, the built site will make use of a client-side index that is created at site build time using [Microsoft Docfind](https://github.com/microsoft/docfind/tree/main).
6. [x] We have a minor issue where when we generate the static build (also in gui and server mode), ffmpeg sends a bunch of warnings to the console (probably to stderr, but that should be confirmed). this likely happens while fetching metadata from videos.  i've been testing in the ~/Documents/Magic directory which produces lots of these.  use that if needed. i don't want any ffmpeg output to show up.
7. [x] Inside a markdown file, links are relative to the file. So a link that has no leading path part (no ../ or /) is in the same folder. But when the markdown file is turned into a url path part, then to reference that other file as a url, we need a leading `../`. When rendering markdown, some relative links need to be adjusted so as not to 404. We're going to need extensive tests on this to make sure we don't create inadvertant 404s
8. [x] We need to level up our search in a couple of ways. First, we need to recognize that different repositories of markdown have different conventions and different frontmatter in the markdown (not always "tags", for example, but could be "keywords" or "taxonomy.tags" or whatever; could also be a "category"). We need to be able to search on arbitrary fields. And to unify the static and dynamic modes and the UI, we should be able to search specific tag fields (facet) using a `key:value` reference. We also need an option to search current folder and subfolders (and current folder should really be the parent of the current markdown file) or everywhere.  We need a second option to specify markdown files or all files. And we need to index other text files and pdfs in a separate index.
9. [x] Right now the file watcher is looking in some folders that can be ignored. Lets make a list of such folders and files to ignore and make that configurable. Defaults should exclude .direnv/, .git/, result/, target/, and build/
10. [x] This is a markdown browser, but right now there's just a long, unstyled list of markdown files and folders at the bottom of the page. Instead, there should be popup, much like with search, but with a styled list of everything. The current mardown file or folder should be highlighted and visible if the screen is small.  On mobile, it should fill the whole screen (with an X in the corner to dismiss) and on desktop it should act more like a sidebar taking up 1/3 to 1/2 of screen width depending on the size of the window.  Descriptions should be added when available.  It should be hidden until a menu item is pressed or a hotkey (`-`) is and can be dismissed with escape or by clicking the X.  Currently there's a hamburger icon and dropdown menu with "browse" and "tags" listed, but that can go away. The browse button can be the hamburger button.  Finally, if tags (or something recognizable as such, eg "tag", "category", "keywords", "taxonomy.tags", etc.) is detected, all discovered values should be shown as little pills and when pills are clicked, the browser should filter down what's shown to just include those tags. Click two tags and assume an "AND".  The tags can be at the top, but with a limited display area and something to click that will expand to show all. Note: the site.json file that's fetched should be used to drive this. Also, that should be a common fetch (this should already be working) so it can be used across other components.
11. [x] next/previous
12. [x] I need to be able to iterate on the UI quicker and without having to rebuild the rust project.  I think there are two ways to do this.  The first is to build out components/index.html to be more like the main template and to use mocks for various things. The trouble here is that it can get out of sync and also I might want to be editing not just the components that are loaded there (and hot reloaded by vite), but also the HTML.  What would be better is if I could run mbr on a live directory, but have it detect when I make changes to HTML or typescript and hot reload.  We already have a hot reloader for when markdown files change, but this is a bit different.  For one thing, we currently compile default templates and css and javascript into the mbr binary -- and I want to keep this behavior.  But I want to be able to run mbr in a dev mode that points to a root dir for the source code and alternately picks the defaults from there live.  So I'd see this as a command line flag like `--dev path/to/source/root` and then we'd need the reloads to happen on changes to javascript and html files there.  We need to make the code clean and generic so it expands easily as we add new files and components.  Finally, when we're in this dev mode, we need to be auto-compiling the components when any of the component sources changes.  I'm not sure it makes sense to do this inside the mbr binary though, so I'm looking for an analysis of approaches and a presentation on them with a recommendation.
13. [x] breadcrumbs
14. [x] Make the UI work for power users.  I want vim-like shortcut keys everywhere. We should be able to scroll the page or the frontmost popup (like search results) with ctrl-f/b and ctrl-d/u.  Where there is a list of results (like search or browse), we should be able to walk through them with j/k as well as arrows.  To launch search, we should be able to use `/`.  Our markdown browsing widget should activate with a simple `-` or `<F2>` if that's possible.
15. [x] On MacOS, when open mbr as an application, it sets the current working directory to the root, `/` and then proceeds to try to crawl over every file on the system. That isn't ideal. We need to handle when it is opened with a file or folder and if the app is opened with neither of these and in the root, it needs to launch a file open dialog for something to be selected, and then startup after a choice is made. there should also be a Open menu under File in the bar. If something is displayed and there's an Open and it goes to a new repo, we essentially have to start over on initialization for repo root, repo details, and everything else.
19. [ ] Bug: the browser shows a markdown file for every folder with an index so we have both a folder and a file when there should just be one.
20. [ ] Let's level up the browser to be a two pane experience (three pane including the currently displayed markdown file).  It should closely mirror the obsidian [notebook navigator plugin](https://notebooknavigator.com) UI and behavior. Research that to get a fuller picture, but when the navigator is activated, the left-most pane will have a tree view of tags (start collapsed) on top followed by a tree view of notes. The notes tree view will show folders and counts of all child markdown notes under it. The tags will also show counts (in gray right aligned) of number of notes for each tag. Use alphabetical sorts.  When a tag or folder is selected, the second pane will show a list of files with filename in gray on top (this deviates from notebook navigator which doesn't show filename at all), a bold and black title, and then the description under that. They also have images, but we'll save that for the future.  Under the description is a list of tags in a pill styling, and under that is the date and immediate parent folder (useful when browsing by tag or when higher up in a hierarchy of folders and viewing all).  All information should be loaded in the single json for the entire site.  When clicking on a markdown note, it should show up in the third pane.  When the navigator is dismissed, just the third pane should be left. Above the Tags header, there should be a "recent files" drop down, driven by last modified dates from the main json file of information, but blended with a browser-based local storage keeping track of recently viewed files. So the 15 most recently viewed and edited will be blended into a unique list of up to 30 files ordered by viewed or edited time.  And finally, there should be a "Shortcuts" list of pinned files that are stored in a browser local storage, but we'll build this UI later, so just setup for it now. We'll also add things like icons and colors to tags and even folders as configuration that can be added in the .mbr/config file, but this will also come as a future phase.  For now we'll use default folder and tag icons and consistent colors. Besides tags, some markdown repositories have other recurring frontmatter elements for organization, which could be things like "category" or "performers" and so forth. If more than 10 notes have such a bit of frontmatter and there is some commonality (so unlike a field like "description" they seem to be used for organization) then they should have selectors, too, similar to tags.
21. [ ] Add search/filter abilities to the note browser.  Allows for fast filtering of navigation with a separate search that prunes empty folders and tags that don't apply and only searches metadata (filename, title, description) using similar syntax to our main search but not allowing for full text search and using this different interface of hierarchical navigation showing just what's relevant.
22. [ ] Add an info panel that uses the info component, which already pops up on the right side but currently doesn't have any information. This will show all of the yaml frontmatter metadata like title, description, tags, date, etc. and any filesystem metadata, too, including full path and filename.  Additionally, we want to have collapsible lists of links out, links in, and a note browser which is a dynamic and hierarchical view of all of the titles in the document  nested to show their level (h2 is indented from h1's, h3's indented from h2's, etc.), but since titles may not be strictly hierarchical, just use the level to determine the indent and list them in a flat list.  This list should be links so that when clicked, you are jumped to an anchor bringing you to the right title in the document.  If we don't already have anchors for each heading, we need to add this as part of the markdown generation. The anchor should be an id that is basically the title lowercased and with dashes removing special chars. BUT, we need to watch out for repeated headlines like ("### Example") and number these so the first one is `#example`, the second is `#example-2`, etc.  The entire region should be scrollable and should disappear on a click. It needs to work well in mobile (take up the whole screen) and in desktop (take up a reasonable amount depending on width of screen, so maybe 20% on a wide screen, maybe 40% of a narrower one). For the drawer, I want it to work even without javascript. In fact, this is my preference for a lot of frontend, when possible.  As a reference, look at daisyui's drawer component which bases itself on a checkbox and then has html that serves as labels for the checkbox. everything else about it is pure css and not too much css either. For colors and styling, mostly stick with defaults, but make use of the current picocss theme and related css variables for accent colors, background colors, etc.
23. [ ] Editing of metadata (tags and other yaml frontmatter) maybe including description recommendations using in-browser local AI for a given note.
24. [ ] Add a command palette, which can be brought up with either `:` or `cmd-shift-k`.  Everything that has a shortcut key including next/previous file, search, browse, etc., should pop up. Use fuzzy search completion to select the desired item. This will also serve as a sort of shortcut help as the title of the action will be on the left and the shortcut key or keys for the action will be shown right aligned in gray. 
25. [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.
26. [ ] Switch up so this is mostly a library and the cli just calls into the public library interface in different ways
27. [ ] quicklook https://developer.apple.com/documentation/QuickLook

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

