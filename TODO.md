# TODO

## What's Next

* [x] .math-inline needs same coloring as code inline @done(2026-04-22 5:10 PM)
* [x] Update lru to 0.17 @done(2026-04-23 11:49 AM)
* [x] Move Configuration reference docs out of CLI doc into their own doc @done(2026-04-23 12:00 PM)
* [x] Add to our info popup: word count (ignoring frontmatter, of course), Readability scores (eg, Flesch-Kincaid scores) @done(2026-04-23 6:06 PM)
* [x] Allow collapse/expand (hide) of sections by clicking on the heading if it isn't already a link. If not a link, then it should have a cursor icon and on click collapse. When in collapsed mode, it should put a `+` icon as a prefix via css to make clear that clicking again will toggle back. We are not changing the HTML by adding div wrappers here. Instead, on click of a heading we will walk over each subsequent element at that same level and add a "sectionhidden" class to each one until we get to another heading of the same level or higher. so if an h3 is clicked on, it will hide all paragraphs, tables, blockquotes, etc., by adding the marker class and any h4's and h5's, too, until it gets to a h3 or h2 or the end of the content.  When toggling back, use the same logic, but remove the sectionhidden classes. This should be implemented as a new component that, when loaded, upgrades the UI, but does it async and without blocking rendering. This new component should also add a link target at the end of each heading, separate from the hide/show, that is a link to that specific header. It should be a muted pico color and look like a link and be a href to the current page with the current anchor based on the ID of the current heading. this way someone can jump straight to a specific section easily. @done(2026-04-23 10:46 PM)
* [ ] mbr bug with `f` links. in website blog, clicking the link works, but clicking the link from f popup doesn't. it adds an extra .. that makes it invalid
* [ ] Watch for "TK" at the start of text blocks and do some kind of styling on the whole block when found. Probably a span with `class="todo"` or something.
* [x] Allow links directly to headings of sections (and copy of the urls) @done(2026-04-23 11:17 PM)
* [x] We do link validation on static site build, but don't currently let the user know if the page they're viewing has broken links on it. Triggering a component that shows when there are page errors would be very useful. It should be next to the "i" icon and should be some sort of error icon that only shows if there are detected problems. We could use this for other issues as well if we think of any.  I think an endpoint for errors, per page, that only works in server/gui modes would allow for async fetching of error info without blocking on initial render. @done(2026-04-23 11:19 PM)

* [ ] CriticMarkup support?
* [ ] Export to PDF
  * Print stylesheet support and light background default (though I guess we could make a dark background PDF).
  * Start with the current page as an option.
  * Also allow a print to PDF for the whole site (essentially taking a doc site and compiling everything into chapters in a single PDF).
  * All of this to live only in the GUI app via menu bar items with cmd-<key> shortcuts.
  * Allow printing while we're at it
  * And printing of the compiled book, too
  * On MacOS, printing would probably be enough because the user could export to PDF, but because this is cross-platform, it would be nice if we can find a good way to do this anywhere.
  * CLI tool should support direct markdown to PDF options, too, including for the "book" mode compiling all markdown listed in a sidebar into a single document.
  * When building a book, start with a full page title page then a page with the table of contents, then the converted markdown in any specified order or default order. Align with the GUI for ordering and labeling.
  * Make sure to handle edge cases like extra long titles.
* [ ] Copy rendered to clipboard
  * Again GUI mode only and via menu bar items. Export the rendered HTML as whatever rich text format is native so it can be pasted into emails.  In this case, make background and text foreground (aside from links and colored items) neutral so it can be pasted into different environments so we don't get white text pasted into a white background email.
  * Copy should assume whole document unless there's a selection

* **Big repo (goodwiki) issues**
  * [ ] In mbr-browser and index pages, we need some limit on the number of things shown (tags, files, etc.) or some sort of pagination
    * [ ] The home page currently shows all pages on the site, which means processing all files before loading the index, which in dynamic mode sucks.
  * [ ] wikilinks and the link checker: underscore-prefixed files (e.g., _...Baby One More Time Tour.md) - files with special chars were renamed with underscores but internal links weren't updated -- none of those work yet. not sure what to do
    * Need to look into the spaces vs. underscores stuff a bit here too
    * Answer: only if we submit PRs to pagefind or switch to something else

* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.

* **Publish**
  * [ ] Publish to a homebrew cask?
  * [ ] Publish to determinate's flake hub?

* [ ] Need to produce robots.txt and sitemap.xml files (robots pulled from .mbr so user can override)? We would need some custom frontmatter to cause something to be left out or even ignored. We also need to use last update or date field to push into sitemap too.  But our "everything is relative" idea falls apart since the sitemap needs to know the full URL of the content (hostname, prefix path, etc.) so maybe we'd only build it if that's specified.

* [ ] Components are currently bundled as mbr-components.js and loaded as a single file, which is great, but we want to allow for more fine-grained overrides.  The better behavior here is for us to assemble a mbr-components.js file from a set of individual files allowing for user overrides to those files.  A static build will have a single mbr-comonents.js file and a dynamic one will concatenate each component file in a particular dir together first checking for per-repo or templates dir overrides.
* [ ] Pull in lightningcss and auto combine and minify the pico.min.css + theme.css + user.css files.

* [ ] Make demo videos
  * Quick highlight reel
  * Demo quicklook
  * Demo simple preview
    * Show live updates
    * Show how it finds images
    * Show how links are fixed automatically
  * Demo markdown supported extensions
  * Demo rich media -- media browser, video, inline pdf
    * Covers, dynamic chapters and captions, and dynamic downscaling, too
  * Demo oembed bare links
  * Demo speed
  * Demo slides
  * Demo tags
  * Demo customizations
  * Demo search and browse
    * Show advanced things like ordering
    * Did I make a hide from browse feature? Should I?
