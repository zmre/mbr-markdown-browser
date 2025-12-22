---
title: README for mbr, the markdown browser
---

# mbr - markdown browser - ALPHA

**THIS IS A WORK IN PROGRESS AND NOT YET STABLE**

The goal of this is to preview markdown under an assumption that there are other markdown files around and we want to be able to jump around between them by following links, browsing tags, browsing folders, and searching. Ultimately, things like backlinks, in-document table of contents, and more will be available.  So first it's a markdown previewer, but then a markdown browser for navigating markdown files. And finally, I want it to be an optional static site generator.

A key principle is that any given repo of markdown files can have the UI for browsing it customized -- not just styling but pretty much everything.  Users customize by creating a `.mbr/` folder in the root of their markdown file project.  Inside that, there can be a `config.toml` file for customizing.  Additionally, all javascript, css, and html components are fetched from a URL that is `/.mbr/something` and it will look in the local `.mbr/` folder for the files before falling back to compiled-in defaults.

Also, some of my notes have a lot of embedded videos in them so I want these to work out of the box without much effort or any ugly syntax.  I'm adding some extensions to standard github flavored markdown to enable these things.

## Technical approach

1. Markdown will convert to HTML on the fly
2. HTML will be served up from a local private web server
3. The UI is HTML+JavaScript+CSS with Lit web components
4. Everything (style, behavior) is highly configurable and selectively overrideable
5. Performance is extremely important -- for launch of GUI and server, render of a markdown, build of a site, and for built sites, loading and rendering in a browser.

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

# Generate static site to custom output
mbr -b --output ./public /path/to/notes
```


## Developing

See [DEVELOP](DEVELOP.md)

## TODO

See [TODO](TODO.md)
