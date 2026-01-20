# TODO

## What's Next

* Static build
  * [ ] Setup some benchmarking and profiling

* UX
  * [ ] Track links out and links in between files
    * Will this be a performance problem?  Right now I just scan the head of each markdown file, but this will require me to read in full files and to markdown process them, too, even when i'm only previewing a single file.  How important are inbound links?  Is there an easier way to quickly grep through files and pull them all out?  Maybe this is a feature that's off by default but can be enabled? 
    * I'd want to have a internal-links.json file as an endpoint that could be fetched (or built in static mode) that simply contained every link from and to which could then be processed over in various ways in the frontend.
      * But alternately, I could make a smaller file (cuz that could get huge) that's per markdown.  I'd still process all links in a static build, but then I'm processing all files anyway.  In a live mode, I'd just grep for links to the current page.  Every page would then have some sort of URL like `/path/to/page/links.json` that would do the search live. And in a built site we'd produce those.  That links.json would have a list of all inbound and outbound links.  But we'd manage generating them in static mode very differently (compile all links in memory then write each links.json file as a build step, if enabled) vs. in live mode where we'd do some sort of grep over markdown files.
      * I think this could perform moderately well for most repos.
  * [ ] In the UI, in addition to adding the link lists to the info bar, I want to be able to press the "F" key and then filter over the links out using fuzzy search on link titles (highlight link selected if hovering or using ctrl-n/p to navigate results list) and I want to be able to hit a toggle key (tab?) to switch to links in and has the same UI.  When the selection window initially pops up, it should show just what links are currently on the screen from top to bottom (or put those ones at the top in the sort order anyway).  And as long as we're doing that, let's add a shortcut to jump to the pages that link inbound (capital `F`) and lets add a third tab that is the table of contents. We can press tab/shift-tab to switch between the tabs or we can go straight to one. Capital "T" should trigger the table of contents one and that should have the same fuzzy search interface.
    * We can actually make this UI widget minus the inbound links right away if we want
  * [ ] Setup configuration allowing a user to specify which metadata should be treated as tags. Default: `["Tags"]`
    * Here's the idea: each item in the list will be treated as a type of tag (case insensitive).  When we have search and filtering, we'll include everything listed that has any values.  In my magic repo, it will be `["taxonomy.tags", "taxonomy.performers"]`. In the wikipedia pages, it would be `["Categories"]` although I foolishly renamed that to Tags in the files so I'll need to rename it back.
      * We'll need to be able to pluralize and singularify it. Keywords/Keyword. Tags/Tag. Categories/Category. Performers/Performer.  So the actual config may need to be a little bit richer.
      * I want any link to be able to target something like: `[blah](Tag:todo)` or wikilink style: `[[Tag:todo]]` and that should link to a landing page for that tag, which will use some html template, but list all the pages with that tag.
      * This will make using the wikipedia stuff nicer as it has a lot of `[[Category:10th_Royal_Hussars_officers]]` style links that are currently giving 404s. But nevermind that, I just like the concept. And I didn't like the idea of using a known special file path to avoid collisions and avoid confusing editors. We can autobuild these landing pages for static sites. Maybe need a way to indicate if these landing pages should be built as part of that tag config with sane defaults.
      * Maybe a convention where all spaces in tags become underscores for organization purposes? Page titles and references can use the spaces?  So `Performer:Joshua Jay` needs to be `Performer:Joshua_Jay` and frontmatter could use either and we'll normalize.
    * And this will let us build out landing pages and links. 

* Big repo (goodwiki) issues
  * [ ] In mbr-browser and index pages, we need some limit on the number of things shown (tags, files, etc.)
  * [ ] wikilinks and the link checker: underscore-prefixed files (e.g., _...Baby One More Time Tour.md) - files with special chars were renamed with underscores but internal links weren't updated -- none of those work yet. not sure what to do
    * Need to look into the spaces vs. underscores stuff a bit here too
    * Answer: only if we submit PRs to pagefind or switch to something else

* Theming
  * [ ] Test html template overrides including partials and includes using the template to see if I can override just a footer and if so, make sure it's documented right
  * [ ] Do I need a different mode that always shows nav and page info when on a wide screen? Maybe a configuration?  And if we have an autoexpanding browser, should we ditch the two column thing and do something more like normal doc sites?  Better: if we could auto-pin those items as a CSS option (like by looking at a CSS var?) that would be awesome.

* [ ] when in server/gui mode and a new file is detected or a file is removed, we need to invalidate our search and browse caches and regenerate our site.json file either entirely or selectively.  i've been running this as a long running server and when i update files, they aren't showing up in the navigation unless i restart the service.

* Browser widget updates
  * [ ] Overhaul the browser widget so it is a single column folder hierarchy. This will remove extra info that's displayed inline, though maybe we can still use that somehow?
  * [ ] Enhance the browser widget to allow more keyboard shortcuts (hjkl for starters)
  * [ ] Enhance the browser widget to have a broader idea of tags and other frontmatter
  * [ ] Bug in browser widget not showing all tags or full counts; also not hiding tags section if there aren't any
  * [ ] Add search/filter abilities to the note browser.  Allows for fast filtering of navigation with a separate search that prunes empty folders and tags that don't apply and only searches metadata (filename, title, description) using similar syntax to our main search but not allowing for full text search and using this different interface of hierarchical navigation showing just what's relevant.

* [ ] Add index pages for frontmatter taxonomy (maybe explicitly defined and requested) like performer, tag, etc. and maybe optionally specify content partials in the .mbr dir?


  * [ ] Add ability to specify code blocks of type mbr-search which will client-side produce search results that are displayed (for static sites, may need to build it out ahead of time, but this would slow things down)

* [ ] Allow print from inside gui mode if we can do that cross platform.

* [ ] Add a command palette, which can be brought up with either `:` or `cmd-shift-k`.  Everything that has a shortcut key including next/previous file, search, browse, etc., should pop up. Use fuzzy search completion to select the desired item. This will also serve as a sort of shortcut help as the title of the action will be on the left and the shortcut key or keys for the action will be shown right aligned in gray. 

* [ ] Editing of metadata (tags and other yaml frontmatter) maybe including description recommendations using in-browser local AI for a given note.

* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.

* [ ] Right now, we have two different types of repos: ones where there's a title in the yaml frontmatter (which should show up as a h1 in the template) and ones where there's just an h1 and no frontmatter. we display the current title at the top of the window, but that assumes the yaml frontmatter approach. and we don't do anything with it if there isn't an h1. so client-side (or in the template language, maybe?), i want to see if there's a defined `title` in frontmatter. if not, i want to set the frontmatter title (in local ram) to the contents of the h1 if it exists. Default fallback is the filename.  That should take care of the title at the top of the gui window. But also, if we have a yaml title but no H1, we should add a H1 at the top of the document with the title field. and to make all this work nicely in built websites, we should probably do some amount of detection when parsing the markdown so we can always have the frontmatter (and therefore the `<head>` metadata) correct even if there's no frontend javascript.  Likewise, we should generate the h1 if a frontmatter title exists but not any existing h1.  if we do this server-side, it will be consistent for built sites as well as live gui/server.

* [ ] Do we want a tui, too? maybe too much bloat?  or maybe awesomesauce?  if we did a tui for showing markdown then it would need to browse and jump around it, too, and have a key for launching an editor.  maybe re-use colors from pico variables in css?  my use case here is for the two linux machines i ssh into. locally i think i'd always just use the gui.  but it may be an awful lot to have gui and tui in one binary so the other option is to make different binaries?  or just feature flags?  thinking needed. (ratatui)

* **Publish**
  * [ ] Publish to crates.io?
  * [ ] Publish to a homebrew cask?
  * [ ] Publish to determinate's flake hub?

* [ ] Need to produce robots.txt and sitemap.xml files (robots pulled from .mbr so user can override)? We would need some custom frontmatter to cause something to be left out or even ignored. We also need to use last update or date field to push into sitemap too.  But our "everything is relative" idea falls apart since the sitemap needs to know the full URL of the content (hostname, prefix path, etc.) so maybe we'd only build it if that's specified.

* [ ] Components are currently bundled as mbr-components.js and loaded as a single file, which is great, but we want to allow for more fine-grained overrides.  The better behavior here is for us to assemble a mbr-components.js file from a set of individual files allowing for user overrides to those files.  A static build will have a single mbr-comonents.js file and a dynamic one will concatenate each component file in a particular dir together first checking for per-repo or templates dir overrides.
* [ ] Pull in lightningcss and auto combine and minify the pico.min.css + theme.css + user.css files.
