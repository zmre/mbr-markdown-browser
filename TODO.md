# TODO

## What's Next

* [ ] Add ability to search also media metadata (filename, title, whatever) beyond just pdfs so we can find videos and such, too.  Need to think about how to display the videos if selected though. Popup up a `<video>` overlay?  Ditto for pictures and audio.  For video, we'd want our usual transcript and chapters type stuff.

* Specify:
  * [ ] Extract first page image from pdf.  Similar to how we do cover images with videos where we have a special filename that triggers the grab, like path/to/mypdf.pdf.cover.png where the "cover.png" is appended to the path of the pdf. do not create these files in static mode, but serve them dynamically upon request in server/gui modes.  as with the video functionality, we want to provide a CLI option to generate a cover image for a specific video that saves the cover image next to the pdf. that will help it work in static mode and also will speed things up in dynamic/gui mode when it exists.
  * [ ] For videos, we want to also extract Genre and Album fields if they exist.
  * [ ] Create a couple of special endpoints for viewing specific media on its own page: /.mbr/videos/?path=whatever and this will be a blank page with normal template stuff and on load, client-side we will make the video html and load in whatever they specified as the video.  We need to make sure our video enhancing component that adds transcript and chapter exploration is also triggered after a video is added.  By making it purely client side it can be the same with static and dynamic.  We want something similar for PDF and for audio files.
  * [ ] Media browser component
    * this is a component that will basically take over 98% of the `<main>` component and it will use site.json to build cards for media -- things that are kind pdf, audio, and video. Each card will attempt to use a cover image, if it exists, or a fallback, and will display as much metadata as it has including the relative path to the media from the repo root or the static folder root.  Each card will link to the /.mbr/videos/?path=whatever style path that is relevant to the media.  cards will be fluid with a min and max width and height and a flexbox wrap
    * we can do images, too, but they will only display the image in small form and clicking on them will just directly link to the image
    * we must have filtering and sorting and we should force the choice of type of media with a default. only show options that exist in the site.json.  filter and sort can be done in-memory client-side based on the site.json data.  for sort, we should have created, updated, and any other relevant metadata that has been filled out. also we should have a text field for filtering that operates off of title and path and filename. build in pagination (with a "more" style link at the bottom when results are more than, say 200)
    * This component will be located within the search component with a "Browse media" link that then launches in a popup dialog that is basically the full width and height of the content area inset a tiny bit to show a border and shadow so it's clear it is on top. there should be an X to dismiss it and escape should also do that. when launched, focus should go into the text filter field.
  * Only partly related, but we want to enhance the mbr-video-extras component to keep track of progress in watched videos. use browser local storage to store off the video url (minus any extra bits after the path to the media starting with `#` or `?`) and the timecode in that file. this is only added or updated on play of a video. later, when a video is loaded, we should look to see if we remember the last play point and, if so, set the playhead there.  but this is important: only do so if that play point is within our start and end boundaries, if we have them. otherwise do nothing, but update the saved state if the video is played.
* Plan
  * [ ] We have a bug with PDF metadata processing. In a big repo of PDFs, not one has any non-null metadata so we need to figure out why that isn't working and fix it
  * [ ] Right now our site.json has a format I'd like to fix, and then fix anything that uses site.json downstream.  In the other files array, there's a "kind" key which can either be "text" or an object with a key of the type and a value with metadata details. For example, `"kind": {"video": {...}}`.  But this is a little awkward. Instead, kind should always have an object which should always have a "type" key.  There are tricks in typescript to setup the types so depending on "type" the other fields differ.  So we want `"kind": { "type": "video", ...}`
  * [ ] Right now .srt files are labeled with a kind of "Other" but they are text. Fix.

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
