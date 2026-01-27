# TODO

## What's Next

* [ ] Add ability to search also media metadata (filename, title, whatever) beyond just pdfs so we can find videos and such, too.  Need to think about how to display the videos if selected though. Popup up a `<video>` overlay?  Ditto for pictures and audio.  For video, we'd want our usual transcript and chapters type stuff.

* **Static build**
  * [ ] Setup some benchmarking and profiling

* **Big repo (goodwiki) issues**
  * [ ] In mbr-browser and index pages, we need some limit on the number of things shown (tags, files, etc.) or some sort of pagination
    * [ ] The home page currently shows all pages on the site, which means processing all files before loading the index, which in dynamic mode sucks.
  * [ ] wikilinks and the link checker: underscore-prefixed files (e.g., _...Baby One More Time Tour.md) - files with special chars were renamed with underscores but internal links weren't updated -- none of those work yet. not sure what to do
    * Need to look into the spaces vs. underscores stuff a bit here too
    * Answer: only if we submit PRs to pagefind or switch to something else

* **Theming**
  * Need a new browser widget
    * [ ] Add search/filter abilities to the note browser.  Allows for fast filtering of navigation with a separate search that prunes empty folders and tags that don't apply and only searches metadata (filename, title, description) using similar syntax to our main search but not allowing for full text search and using this different interface of hierarchical navigation showing just what's relevant.

* [ ] Editing of metadata (tags and other yaml frontmatter) maybe including description recommendations using in-browser local AI for a given note.

* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.

* **Publish**
  * [ ] Publish to crates.io?
  * [ ] Publish to a homebrew cask?
  * [ ] Publish to determinate's flake hub?

* [ ] Need to produce robots.txt and sitemap.xml files (robots pulled from .mbr so user can override)? We would need some custom frontmatter to cause something to be left out or even ignored. We also need to use last update or date field to push into sitemap too.  But our "everything is relative" idea falls apart since the sitemap needs to know the full URL of the content (hostname, prefix path, etc.) so maybe we'd only build it if that's specified.

* [ ] Components are currently bundled as mbr-components.js and loaded as a single file, which is great, but we want to allow for more fine-grained overrides.  The better behavior here is for us to assemble a mbr-components.js file from a set of individual files allowing for user overrides to those files.  A static build will have a single mbr-comonents.js file and a dynamic one will concatenate each component file in a particular dir together first checking for per-repo or templates dir overrides.
* [ ] Pull in lightningcss and auto combine and minify the pico.min.css + theme.css + user.css files.

* [ ] Make demo videos
