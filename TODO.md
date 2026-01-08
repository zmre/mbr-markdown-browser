# TODO

## What's Next

* [x] In my blogs and elsewhere, I use `>` for quotes (styled with a gray left border and some indent), `>>` for pull quotes that are bigger, in a magazine pull quote style.  This should all be done, if possible, purely with CSS and not with the production of special HTML.  The block quote styling right now is fine, but the double pull quote needs new styling.
  * For pull quotes, see this blog: https://ironcorelabs.com/blog/2025/human-error-data-breaches/ which uses a larger font, a background color and the left border. Italic and bold, too.
  * Next we want to style alerts, which are a [github markdown extension](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax#alerts). When the first line of a blockquote is `[!NOTE]` or `[!TIP]` or `[!IMPORTANT]` or `[!WARNING]` or `[!CAUTION]`, then an alert block is created, which is just a regular blockquote but with a class like `markdown-alert-note` added on.  We need to style these.
    * For visual styling concepts for the tip boxes, see these tailwind css alerts boxes as examples. they use colored icons, color border, color background, and color text where they are all different shades of something. For example, the warning uses a yellow exclamation mark icon, faint muted darker yellow background, slightly less muted yellow border, and bright yellow text on top for good contrast.  https://tailwindcss.com/plus/ui-blocks/application-ui/feedback/alerts
    * Additionally, in keeping with github standard alerts, there should be a title on the box (added purely with CSS) that has an icon and the type, like "Note" or "Tip", etc.  Note is blue, tip is green, important is purple, warning is orange or yellow, and caution is red, though all colors should be tied to pico-css variables, if possible.
  * Finally, we want to make a way to add marginalia, which is an aside set out of the flow of text into the margin.  We can do this when blockquote is three deep (`>>>`).


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
* [ ] quicklook https://developer.apple.com/documentation/QuickLook
* [ ] **Navigation**
  * [ ] Track links out and links in between files
* Videos
	* [ ] Serve captions, chapters, and posters automatically when in server/gui mode and when the relevant files don't exist already; use ffmpeg to dynamically extract and serve
  * [ ] Add chapter and caption tracks when available
  * [ ] Enhance the UI to allow caption and chapter expansion outside of the video window and to jump to the appropriate place in the video on click inside them, plus tracking for where we are so the appropriate caption or title is shown when those bits are expanded.
* [ ] Make all links relative so for example from `/xyz` to `../../xyz` as needed which will handle static generation hosted mode and prefixes and more
  * All links that are relative will need to be converted 
  * in arbitrary subfolders
  * Also, don't allow `..` paths that go outside of the root
  * Hmmm, what about those `/.mbr/whatever` files?  I'd have to change those differently on every page -- how to do this in the template?  ie, not just from markdown so maybe **need to post process the output html**


