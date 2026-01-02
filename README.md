# mbr - markdown browser

**THIS IS A WORK IN PROGRESS AND NOT YET STABLE** but testers are welcome.

This is inspired by tools like [Marked 2](https://marked2app.com), [Zola](https://www.getzola.org), [mdbook](https://github.com/rust-lang/mdBook), and [Obsidian Publish](https://obsidian.md/publish).

The goal of this is to preview markdown under an assumption that there are other markdown files around and we want to be able to jump around between them by following links, browsing tags, browsing folders, and searching. Ultimately, things like backlinks, in-document table of contents, and more will be available.  So first it's a markdown previewer, but then a markdown browser for navigating markdown files. And finally, I want it to be an optional static site generator.

A key principle is that any given repo of markdown files can have its UI/UX be customizable including styles, themes, and even functionality.  Users customize by creating a `.mbr/` folder in the root of their markdown file project.  Inside that, there can be a `config.toml` file for customizing, though it isn't required.  All javascript, css, and html components are fetched from a URL that is `/.mbr/something` and it will look in the local `.mbr/` folder for the files before falling back to compiled-in defaults. There's also a command-line flag to look in an alternate location.

Another key principle is speed: rendering markdown should be almost instantaneous.  Viewing a markdown file should launch and render quickly, with progressive enhancement for other features involving search and browsing.

I've added quite a few minor markdown extensions. For example, if an image link (`![description](path/to/image)`) points to something other than an image, like an audio file, video file, or pdf, then we will provide the appropriate embedding. Bare links on their own lines will be enriched oembed-style.  Because some of my note repos have a lot of other assets, managing these is also of great importance.

While I want to use this first and foremost as a markdown previewer with live updating, and second as a markdown browser, the whole thing is HTML-centric. This means that it is a short extra distance to be able to generate a static website and because there are no other static site generators that meet my various criteria (ability to blend assets with markdown in the same folders, no required directory structures, enriched markdown, fast, handling of videos, and more).

## Technical approach

1. Markdown will convert to HTML on the fly
2. HTML will be served up from a local private web server
3. The UI is HTML+JavaScript+CSS with Lit web components
4. Everything (style, behavior) is highly configurable and selectively overrideable
5. Performance is extremely important -- for launch of GUI and server, render of a markdown, build of a site, and for built sites, loading and rendering in a browser.
6. Leaves nothing behind (no caches, temp files, etc.), unless there's a static build and even then, only in the build directory.

## Installation

### Using Nix (Recommended)

```bash
# Run directly without installing
nix run github:zmre/mbr -- -s /path/to/your/notes

# Build and install
nix build github:zmre/mbr
./result/bin/mbr -g /path/to/your/notes

# Add to your flake
{
  inputs.mbr.url = "github:zmre/mbr";
}
# Then use inputs.mbr.packages.${system}.default
```

### Using Cargo

```bash
cargo install --git https://github.com/zmre/mbr
```

## Running

Specify a markdown file or directory to start:

```bash
# Print rendered HTML to stdout
mbr README.md

# Start web server at http://127.0.0.1:5200/
mbr -s /path/to/notes

# Launch native GUI window with web server
mbr -g /path/to/notes

# Generate static site to ./build folder
mbr -b /path/to/notes

# Generate static site to custom output folder
mbr -b --output ./public /path/to/notes
```


## Developing

See [DEVELOP](DEVELOP.md)

## TODO

See [TODO](TODO.md)
