# TODO

## What's Next

* **Static build**
  * [ ] Setup some benchmarking and profiling

* **UX**
  * [ ] I don't think .mbr config overrides are being used in quicklook. i need to test html partials, but the theme= change doesn't seem to be honored.  Additionally, when resolving asset links, quicklook doesn't seem to take into account config (even the default config) for static files.  mbr will show images in a static folder while quicklook shows broken images when viewing the same file.

* **Big repo (goodwiki) issues**
  * [ ] In mbr-browser and index pages, we need some limit on the number of things shown (tags, files, etc.) or some sort of pagination
    * [ ] The home page currently shows all pages on the site, which means processing all files before loading the index, which in dynamic mode sucks.
  * [ ] wikilinks and the link checker: underscore-prefixed files (e.g., _...Baby One More Time Tour.md) - files with special chars were renamed with underscores but internal links weren't updated -- none of those work yet. not sure what to do
    * Need to look into the spaces vs. underscores stuff a bit here too
    * Answer: only if we submit PRs to pagefind or switch to something else

* **Theming**
  * [ ] Test html template overrides including partials and includes using the template to see if I can override just a footer and if so, make sure it's documented right
  * [ ] Do I need a different mode that always shows nav and page info when on a wide screen? Maybe a configuration?  And if we have an autoexpanding browser, should we ditch the two column thing and do something more like normal doc sites?  Better: if we could auto-pin those items as a CSS option (like by looking at a CSS var?) that would be awesome.
  * Need a new browser widget
    * [ ] Overhaul the browser widget so it is a single column folder hierarchy. This will remove extra info that's displayed inline, though maybe we can still use that somehow?
    * [ ] Enhance the browser widget to allow more keyboard shortcuts (hjkl for starters)
    * [ ] Enhance the browser widget to have a broader idea of tags and other frontmatter
    * [ ] Bug in browser widget not showing all tags or full counts; also not hiding tags section if there aren't any
    * [ ] Add search/filter abilities to the note browser.  Allows for fast filtering of navigation with a separate search that prunes empty folders and tags that don't apply and only searches metadata (filename, title, description) using similar syntax to our main search but not allowing for full text search and using this different interface of hierarchical navigation showing just what's relevant.

* [ ] when in server/gui mode and a new file is detected or a file is removed, we need to invalidate our search and browse caches and regenerate our site.json file either entirely or selectively.  i've been running this as a long running server and when i update files, they aren't showing up in the navigation unless i restart the service.


* [ ] Add ability to specify code blocks of type mbr-search which will client-side produce search results that are displayed (for static sites, may need to build it out ahead of time, but this would slow things down)

* [ ] Allow print from inside gui mode if we can do that cross platform.

* [ ] Editing of metadata (tags and other yaml frontmatter) maybe including description recommendations using in-browser local AI for a given note.

* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.

* [ ] Do we want a tui, too? maybe too much bloat?  or maybe awesomesauce?  if we did a tui for showing markdown then it would need to browse and jump around it, too, and have a key for launching an editor.  maybe re-use colors from pico variables in css?  my use case here is for the two linux machines i ssh into. locally i think i'd always just use the gui.  but it may be an awful lot to have gui and tui in one binary so the other option is to make different binaries?  or just feature flags?  thinking needed. (ratatui)

* **Publish**
  * [ ] Publish to crates.io?
  * [ ] Publish to a homebrew cask?
  * [ ] Publish to determinate's flake hub?

* [ ] Need to produce robots.txt and sitemap.xml files (robots pulled from .mbr so user can override)? We would need some custom frontmatter to cause something to be left out or even ignored. We also need to use last update or date field to push into sitemap too.  But our "everything is relative" idea falls apart since the sitemap needs to know the full URL of the content (hostname, prefix path, etc.) so maybe we'd only build it if that's specified.

* [ ] Components are currently bundled as mbr-components.js and loaded as a single file, which is great, but we want to allow for more fine-grained overrides.  The better behavior here is for us to assemble a mbr-components.js file from a set of individual files allowing for user overrides to those files.  A static build will have a single mbr-comonents.js file and a dynamic one will concatenate each component file in a particular dir together first checking for per-repo or templates dir overrides.
* [ ] Pull in lightningcss and auto combine and minify the pico.min.css + theme.css + user.css files.

* [ ] Make demo videos
