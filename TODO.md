# TODO

## What's Next

* Browser widget updates
  * [ ] Enhance the browser widget to allow more keyboard shortcuts
  * [ ] Enhance the browser widget to have a broader idea of tags and other frontmatter
  * [ ] Bug in browser widget not showing all tags or full counts
  * [ ] Bug in browser widget where when all markdown in the root, the Notes section shows nothing under it
  * [ ] Add search/filter abilities to the note browser.  Allows for fast filtering of navigation with a separate search that prunes empty folders and tags that don't apply and only searches metadata (filename, title, description) using similar syntax to our main search but not allowing for full text search and using this different interface of hierarchical navigation showing just what's relevant.
* [ ] I need a showcase for the README to convey what it can do and show demonstrations of it. Might consider setting up some website examples, too?  Github hosted?
* [ ] Add a command palette, which can be brought up with either `:` or `cmd-shift-k`.  Everything that has a shortcut key including next/previous file, search, browse, etc., should pop up. Use fuzzy search completion to select the desired item. This will also serve as a sort of shortcut help as the title of the action will be on the left and the shortcut key or keys for the action will be shown right aligned in gray. 
* [ ] Editing of metadata (tags and other yaml frontmatter) maybe including description recommendations using in-browser local AI for a given note.
* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.
* [ ] Switch up so this is mostly a library and the cli just calls into the public library interface in different ways
* [x] quicklook https://developer.apple.com/documentation/QuickLook
  * [ ] quicklook bugs: videos aren't working inside quicklook preview.
* [ ] **Navigation**
  * [ ] Track links out and links in between files
* Videos
	* [ ] Serve captions, chapters, and posters automatically when in server/gui mode and when the relevant files don't exist already; based on config, use ffmpeg to dynamically extract and serve chapters and captions if they're available inside a video
  * [ ] Enhance the UI to allow caption and chapter expansion outside of the video window and to jump to the appropriate place in the video on click inside them, plus tracking for where we are so the appropriate caption or title is shown when those bits are expanded.
  * [ ] dynamically scale down videos streaming to mobile without pre transcoding them? i'm using rust and axum to serve the videos
* [ ] Make all links relative so for example from `/xyz` to `../../xyz` as needed which will handle static generation hosted mode and prefixes and more
  * All links that are relative will need to be converted 
  * in arbitrary subfolders
  * Also, don't allow `..` paths that go outside of the root
  * Hmmm, what about those `/.mbr/whatever` files?  I'd have to change those differently on every page -- how to do this in the template?  ie, not just from markdown so maybe **need to post process the output html**

* [ ] Right now, we have two different types of repos: ones where there's a title in the yaml frontmatter (which should show up as a h1 in the template) and ones where there's just an h1 and no frontmatter. we display the current title at the top of the window, but that assumes the yaml frontmatter approach. and we don't do anything with it if there isn't an h1. so client-side (or in the template language, maybe?), i want to see if there's a defined `title` in frontmatter. if not, i want to set the frontmatter title (in local ram) to the contents of the h1 if it exists. Default fallback is the filename.  That should take care of the title at the top of the gui window. But also, if we have a yaml title but no H1, we should add a H1 at the top of the document with the title field. and to make all this work nicely in built websites, we should probably do some amount of detection when parsing the markdown so we can always have the frontmatter (and therefore the `<head>` metadata) correct even if there's no frontend javascript.  Likewise, we should generate the h1 if a frontmatter title exists but not any existing h1.  if we do this server-side, it will be consistent for built sites as well as live gui/server.

* [ ] Do we want a tui, too? maybe too much bloat?  or maybe awesomesauce?  if we did a tui for showing markdown then it would need to browse and jump around it, too, and have a key for launching an editor.  maybe re-use colors from pico variables in css?  my use case here is for the two linux machines i ssh into. locally i think i'd always just use the gui.  but it may be an awful lot to have gui and tui in one binary so the other option is to make different binaries?  or just feature flags?  thinking needed.
