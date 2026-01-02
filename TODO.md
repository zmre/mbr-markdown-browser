# TODO

## What's Next

* [ ] Setup github actions, push to cachix, and release cutting; version in cargo.toml, flake.nix and git tags need to line up. Need to figure out how to build and share, too.  Maybe see about code signing and releases that are signed?
* [ ] We need to change up the info sidebar in a few ways:
  * Get rid of the big blue blob in the lower right and make an i icon in the header instead
  * Make a shortcut key for launching the info sidebar
  * It doesn't seem to show all the frontmatter for a given file now -- it seems like it did, but isn't anymore or perhaps sometimes it doesn't show?
  * On a wider screen, the sidebar should be made wider
* Browser widget updates
  * [ ] Enhance the browser widget to allow more keyboard shortcuts
  * [ ] Enhance the browser widget to have a broader idea of tags and other frontmatter
  * [ ] Bug in browser widget not showing all tags or full counts
  * [ ] Bug in browser widget where when all markdown in the root, the Notes section shows nothing under it
  * [ ] Add search/filter abilities to the note browser.  Allows for fast filtering of navigation with a separate search that prunes empty folders and tags that don't apply and only searches metadata (filename, title, description) using similar syntax to our main search but not allowing for full text search and using this different interface of hierarchical navigation showing just what's relevant.
* [ ] Add a command palette, which can be brought up with either `:` or `cmd-shift-k`.  Everything that has a shortcut key including next/previous file, search, browse, etc., should pop up. Use fuzzy search completion to select the desired item. This will also serve as a sort of shortcut help as the title of the action will be on the left and the shortcut key or keys for the action will be shown right aligned in gray. 
* [ ] Editing of metadata (tags and other yaml frontmatter) maybe including description recommendations using in-browser local AI for a given note.
* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.
* [ ] Switch up so this is mostly a library and the cli just calls into the public library interface in different ways
* [ ] quicklook https://developer.apple.com/documentation/QuickLook


## Full List

* Videos
	* [ ] Serve captions, chapters, and posters automatically

* Markdown parsing
  * [ ] Allow unordered bullets under ordered and vice versa
	* [ ] Make all links relative so for example from `/xyz` to `../../xyz` as needed which will handle static generation hosted mode and prefixes and more
        * All links that are relative will need to be converted 
        * in arbitrary subfolders
        * Also, don't allow `..` paths that go outside of the root
		* Hmmm, what about those `/.mbr/whatever` files?  I'd have to change those differently on every page -- how to do this in the template?  ie, not just from markdown so maybe **need to post process the output html**
	* [ ] **Navigation**
		* [ ] Add `link rel="next"` and `link rel="prev"` links in the header and provide next/prev vars to the template
		* [ ] Breadcrumb var?
		* [ ] Track links out and links in between files
* Misc
  * [ ] Make a quicklook plugin that shows this!  That would be epic. Might need to inline all the dependencies?

