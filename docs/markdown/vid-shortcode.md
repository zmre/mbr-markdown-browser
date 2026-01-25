---
title: Vid Shortcode
description: Advanced video embedding with timestamps and captions
---

# Vid Shortcode

For advanced video features beyond standard image syntax, use the `{{ vid(...) }}` shortcode.

## When to Use This

Use the vid shortcode when you need:

- Videos from the `/videos/` folder with automatic path handling
- Explicit start/end timestamps with named parameters
- Custom figure captions
- Spaces in video paths (automatically URL-encoded)

For simple video embedding, the [standard image syntax](./media.md#video-embedding) (`![caption](video.mp4)`) is usually sufficient.

## Basic Usage

```markdown
{{ vid(path="demo.mp4") }}
```

This renders a video player with:

- The video from `/videos/demo.mp4`
- Automatic poster image detection (`demo.mp4.cover.png`)
- Automatic caption/chapter track detection
- HTML5 video controls

## Parameters

| Parameter | Description | Required | Example |
|-----------|-------------|----------|---------|
| `path` | Video path relative to `/videos/` folder | **Yes** | `"demo.mp4"` |
| `start` | Start timestamp in seconds | No | `"10"` |
| `end` | End timestamp in seconds | No | `"30"` |
| `caption` | Figure caption text | No | `"Product demo"` |

## Examples

### With Timestamps

Play only a portion of the video:

```markdown
{{ vid(path="presentation.mp4", start="10", end="30") }}
```

This creates a video that starts at 10 seconds and ends at 30 seconds.

### With Caption

Add a descriptive caption:

```markdown
{{ vid(path="tutorial.mp4", caption="Step-by-step installation guide") }}
```

### All Parameters

```markdown
{{ vid(path="talk.mp4", start="120", end="180", caption="The key insight from the presentation") }}
```

### Paths with Spaces

The shortcode handles paths with spaces automatically:

```markdown
{{ vid(path="Eric Jones/Eric Jones - Metal 3.mp4") }}
```

Spaces are URL-encoded (`%20`) automatically.

## Sidecar Files

The vid shortcode automatically looks for these companion files next to your video:

```
videos/
├── demo.mp4
├── demo.mp4.cover.png         # Poster/thumbnail image
├── demo.mp4.captions.en.vtt   # English captions
└── demo.mp4.chapters.en.vtt   # Chapter markers
```

All sidecar files are optional but enhance the viewing experience when present.

### Extracting Metadata

Use the `--extract-video-metadata` CLI option to generate sidecar files from embedded video data:

```bash
mbr --extract-video-metadata videos/demo.mp4
```

This extracts (if available):

- Cover image from embedded artwork
- Chapters from chapter markers
- Captions from subtitle tracks

## Supported Video Formats

The shortcode supports the same formats as standard video embedding:

- MP4 (`.mp4`) - recommended
- M4V (`.m4v`)
- MKV (`.mkv`)
- MOV (`.mov`)
- AVI (`.avi`)
- OGV/OGG (`.ogv`, `.ogg`)
- MPEG (`.mpg`)

## Smart Quote Handling

Pulldown-cmark's smart punctuation may convert `"` to curly quotes (`"` `"`). The shortcode supports both:

```markdown
{{ vid(path="demo.mp4") }}        <!-- straight quotes -->
{{ vid(path="demo.mp4") }}        <!-- curly quotes (converted) -->
```

## Comparison with Image Syntax

| Feature | Image Syntax | Vid Shortcode |
|---------|--------------|---------------|
| Basic playback | ✓ | ✓ |
| Timestamps | `#t=10,30` in URL | Named params |
| Caption | Alt text | `caption` param |
| Path encoding | Manual | Automatic |
| Videos folder | Any path | `/videos/` prefix |
| Sidecar detection | ✓ | ✓ |

Choose image syntax for simplicity, vid shortcode for explicit control.

## See Also

- [Media Embedding](./media.md) - Standard image syntax for all media types
- [Video Transcoding](/reference/cli.md#video-transcoding) - Dynamic HLS transcoding (experimental)
