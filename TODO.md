# TODO

## What's Next

* Static build
  * [x] Fix / understand slow performance on static builds in big repos; worse, there's zero info on what it's doing while waiting for it even with -vvv
  * [x] Fix / understand slow initial rendering in some markdown files in dynamic mode (I think this is the oembed stuff)
    * [x] Are we parallelizing the oembed stuff?  Can we?
    * [x] Do we have an oembed cache like for when we are doing a static build? Should we?
    * [-] Is there a way to display and then enrich in the case of dynamic viewing? Or to stream?
    * [-] Do we need/want a loading indicator if the above don't work?
    * [x] Can we disable oembed?  Maybe with a value of zero?
  * [ ] Setup some benchmarking and profiling
  * [x] Fix search relevance/ranking for static build site search; it's treating all fields the same or maybe only searching content. i want to prioritize titles and filenames and other metadata over content so i get more relevant results returned

* Theming
  * [ ] Test html template overrides including partials and includes using the template to see if I can override just a footer and if so, make sure it's documented right
  * [x] Add empty partials in a few places so people can extend without overwriting
  * [ ] Light mode issues with color on background stuff (hamburger icon, tag counts, etc.)
  * [x] I messed up. I want regular and regular fluid versions of pico, not the classless stuff.  Regular has classless and classes.
  * [ ] Make the oembed stuff even better with images -- medium style so make a card with header, description, and image if available, which should look nice when oembed enrichment is available
  * [ ] In mbr-browser and index pages, we need some limit on the number of things shown (tags, files, etc.)

* hljs
  * [ ] Make the code syntax coloring be a lit component and have it only load the scripts needed for languages on the page. Also support some base set of languages natively, but load from CDN for the ones we don't bake in

* Browser widget updates
  * [ ] Enhance the browser widget to allow more keyboard shortcuts (hjkl for starters)
  * [ ] Enhance the browser widget to have a broader idea of tags and other frontmatter
  * [ ] Bug in browser widget not showing all tags or full counts; also not hiding tags section if there aren't any
  * [ ] Add search/filter abilities to the note browser.  Allows for fast filtering of navigation with a separate search that prunes empty folders and tags that don't apply and only searches metadata (filename, title, description) using similar syntax to our main search but not allowing for full text search and using this different interface of hierarchical navigation showing just what's relevant.

* [ ] Add index pages for frontmatter taxonomy (maybe explicitly defined and requested) like performer, tag, etc. and maybe optionally specify content partials in the .mbr dir?
  * [ ] Add ability to specify code blocks of type mbr-search which will client-side produce search results that are displayed (for static sites, may need to build it out ahead of time, but this would slow things down)
  * [ ] I want to make docs for this using this, but docs typically have full-time sidebars on wide screens.  Should I make an option that pins this on?  Or is there a way to do that with just CSS?

* [ ] Add tooltips to all hrefs that show the URL they go to.  Alternately setup some js that makes a sort of status at the bottom of the screen showing where a link goes to when hovering.  What about for touch screens?  Is there a click and hold action of some kind or something we can do so a person can know a URL?  And finally, while we're at it, do we want a different styling (prefix icon?) for external links versus internal links?  I think so. I think maybe a subtle globe icon for links that start with http.  And I think that can be done entirely in css.

* [ ] Add a command palette, which can be brought up with either `:` or `cmd-shift-k`.  Everything that has a shortcut key including next/previous file, search, browse, etc., should pop up. Use fuzzy search completion to select the desired item. This will also serve as a sort of shortcut help as the title of the action will be on the left and the shortcut key or keys for the action will be shown right aligned in gray. 

* [ ] Editing of metadata (tags and other yaml frontmatter) maybe including description recommendations using in-browser local AI for a given note.

* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.

* **Navigation**
  * [ ] Track links out and links in between files

* **Videos**
  * [x] Enhance the UI to allow caption and chapter expansion outside of the video window and to jump to the appropriate place in the video on click inside them, plus tracking for where we are so the appropriate caption or title is shown when those bits are expanded.
  * [ ] in the video js component, when the transcript is being shown, make it so clicking on a line of text takes you to the relevant point in the video.  The cursor can change, but I don't want there to be any visual clues (underlines or dotted underlines or blue colors) that the text is clickable. Make sure to update the docs to explain the function.
	* [ ] Serve captions, chapters, and posters automatically when in server/gui mode and when the relevant files don't exist already; based on config, use ffmpeg to dynamically extract and serve chapters and captions if they're available inside a video
  * [ ] dynamically scale down videos streaming to mobile without pre transcoding them? i'm using rust and axum to serve the videos
  * [ ] Intermingle chapter headings into the transcript with some extra styling (when available)

* [ ] Right now, we have two different types of repos: ones where there's a title in the yaml frontmatter (which should show up as a h1 in the template) and ones where there's just an h1 and no frontmatter. we display the current title at the top of the window, but that assumes the yaml frontmatter approach. and we don't do anything with it if there isn't an h1. so client-side (or in the template language, maybe?), i want to see if there's a defined `title` in frontmatter. if not, i want to set the frontmatter title (in local ram) to the contents of the h1 if it exists. Default fallback is the filename.  That should take care of the title at the top of the gui window. But also, if we have a yaml title but no H1, we should add a H1 at the top of the document with the title field. and to make all this work nicely in built websites, we should probably do some amount of detection when parsing the markdown so we can always have the frontmatter (and therefore the `<head>` metadata) correct even if there's no frontend javascript.  Likewise, we should generate the h1 if a frontmatter title exists but not any existing h1.  if we do this server-side, it will be consistent for built sites as well as live gui/server.

* [ ] Do we want a tui, too? maybe too much bloat?  or maybe awesomesauce?  if we did a tui for showing markdown then it would need to browse and jump around it, too, and have a key for launching an editor.  maybe re-use colors from pico variables in css?  my use case here is for the two linux machines i ssh into. locally i think i'd always just use the gui.  but it may be an awful lot to have gui and tui in one binary so the other option is to make different binaries?  or just feature flags?  thinking needed. (ratatui)

* **Publish**
  * [ ] Publish to crates.io?
  * [ ] Publish to a homebrew cask?
  * [ ] Publish to determinate's flake hub?

* [ ] Support for wikilinks?  If we don't have to search for titles, maybe we assume that what's in `[[title]]` is a filename like. There are also links to headings (see https://help.obsidian.md/links) but they allow spaces and stuff so would need to convert to ids.
  * UPDATE: looks like we maybe already support this?  Need to test, verify, and if so, document

* [ ] Need to produce robots.txt and sitemap.xml files (robots pulled from .mbr so user can override)? We would need some custom frontmatter to cause something to be left out or even ignored. We also need to use last update or date field to push into sitemap too.  But our "everything is relative" idea falls apart since the sitemap needs to know the full URL of the content (hostname, prefix path, etc.) so maybe we'd only build it if that's specified.

* [ ] Components are currently bundled as mbr-components.js and loaded as a single file, which is great, but we want to allow for more fine-grained overrides.  The better behavior here is for us to assemble a mbr-components.js file from a set of individual files allowing for user overrides to those files.  A static build will have a single mbr-comonents.js file and a dynamic one will concatenate each component file in a particular dir together first checking for per-repo or templates dir overrides.

