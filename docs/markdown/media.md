---
title: Media Embedding
description: Embed videos, audio, and PDFs in markdown
---

# Media Embedding

mbr extends the standard image syntax to support rich media embedding.

## Image Syntax for Media

The standard image syntax `![caption](url)` auto-detects media types:

```markdown
![My image](photo.jpg)      <!-- Renders as image -->
![My video](demo.mp4)       <!-- Renders as video player -->
![My audio](podcast.mp3)    <!-- Renders as audio player -->
![My PDF](report.pdf)       <!-- Renders as PDF viewer -->
```

mbr detects the type from the file extension.

## Video Embedding

### Supported Formats

- MP4 (`.mp4`)
- MPEG (`.mpg`, `.mpeg`)
- AVI (`.avi`)
- OGV (`.ogv`)
- OGG (`.ogg`)
- M4V (`.m4v`)
- WebM (`.webm`)

### Basic Video

```markdown
![Demo](demo.mp4)
```

Renders a native HTML5 video player with:
- Play/pause controls
- Volume control
- Fullscreen button
- Progress bar

### Video with Timestamp

Specify start and end times:

```markdown
![Highlight](video.mp4#t=10,30)
```

Format: `#t=START,END` where times are in seconds.

More examples:
```markdown
![Start at 1:30](video.mp4#t=90)
![From 10s to 45s](video.mp4#t=10,45)
```

### Automatic Caption/Chapter Detection

If present alongside the video, mbr auto-detects:

```
videos/
├── demo.mp4
├── demo.mp4.captions.en.vtt    # Auto-loaded as captions
└── demo.mp4.chapters.en.vtt    # Auto-loaded as chapters
```

### Interactive Transcript

When captions are available, a "Show transcript" toggle appears below the video. The transcript displays all caption text with visual indicators:

- **Active line** - Highlighted background showing current playback position
- **Past lines** - Slightly dimmed for lines already played
- **Auto-scroll** - Transcript automatically scrolls to keep the active line centered

Clicking any line in the transcript jumps the video to that point and starts playback. The cursor changes to a pointer on hover, but there are no other visual indicators that the text is clickable.

## Audio Embedding

### Supported Formats

- MP3 (`.mp3`)
- WAV (`.wav`)
- OGG (`.ogg`)
- FLAC (`.flac`)
- AAC (`.aac`)
- M4A (`.m4a`)
- WebM (`.webm`)

### Basic Audio

```markdown
![Podcast Episode](episode.mp3)
```

Renders an HTML5 audio player with:
- Play/pause
- Progress bar
- Volume control
- Time display

## YouTube Embedding

### Automatic Detection

YouTube URLs in image syntax become embedded players:

```markdown
![My Talk](https://www.youtube.com/watch?v=dQw4w9WgXcQ)
```

### Supported URL Formats

```markdown
![](https://www.youtube.com/watch?v=VIDEO_ID)
![](https://youtu.be/VIDEO_ID)
![](https://www.youtube.com/embed/VIDEO_ID)
```

All render as responsive embedded iframe players.

### Privacy-Enhanced Mode

YouTube embeds use `youtube-nocookie.com` by default for better privacy.

## Giphy Embedding

Giphy URLs on their own line are automatically embedded as animated GIFs.

### Supported URL Formats

Both the Giphy page URL and direct media URLs work:

```markdown
https://giphy.com/gifs/season-17-the-simpsons-17x6-xT5LMB2WiOdjpB7K4o

https://media.giphy.com/media/xT5LMB2WiOdjpB7K4o/giphy.gif
```

Both render the same animated GIF:

https://giphy.com/gifs/season-17-the-simpsons-17x6-xT5LMB2WiOdjpB7K4o

### How It Works

- **Page URLs** (`giphy.com/gifs/...`) - mbr extracts the ID and converts to a media URL
- **Media URLs** (`media.giphy.com/...`, `i.giphy.com/...`) - embedded directly

No network fetch is required - Giphy URLs are detected and rendered instantly.

## PDF Embedding

```markdown
![Report](document.pdf)
```

Renders an inline PDF viewer using the browser's built-in PDF support.

### PDF Sizing

PDFs render at full width with a reasonable height. Customize with CSS:

```css
/* .mbr/user.css */
object[type="application/pdf"] {
  height: 800px;
}
```

## OpenGraph Link Enrichment

Bare URLs on their own line get enriched with metadata:

```markdown
Some text here.

https://example.com/article

More text here.
```

mbr fetches the URL's OpenGraph metadata:
- Title
- Description
- Preview image

And renders a rich preview card.

### Requirements

- URL must be on its own line
- Blank lines before and after
- URL must be accessible (timeout: 300ms default)

### Timeout Configuration

```bash
mbr -s --oembed-timeout-ms 1000 ~/notes  # 1 second timeout
```

Or in config:

```toml
# .mbr/config.toml
oembed_timeout_ms = 1000
```

### Disabling OpenGraph Fetching

Set timeout to 0 to disable OpenGraph fetching entirely:

```bash
mbr -s --oembed-timeout-ms 0 ~/notes
```

```toml
# .mbr/config.toml
oembed_timeout_ms = 0
```

With oembed disabled, bare URLs render as plain links. YouTube and Giphy embeds still work since they don't require network calls.

## File Organization

### Relative Paths

Media paths are relative to the markdown file:

```
docs/
├── guide.md           # ![](./images/diagram.png)
└── images/
    └── diagram.png
```

### Static Folder

Files in the `static/` folder (configurable) are served directly:

```
notes/
├── static/
│   └── videos/
│       └── demo.mp4
└── guide.md           # ![](videos/demo.mp4)
```

This concept is so mbr can work well when navigating sites setup for other static builders like zola, astro, etc.  Assets in the specified static folder, if any, will be overlaid with the markdown.  However, you don't have to put anything in a static folder.  Images and media can be intermingled with notes.

### Absolute Paths

Use leading slash for root-relative paths:

```markdown
![Logo](/images/logo.png)    <!-- Always from repository root -->
```

## Performance Tips

### Video

- Use `.mp4` with H.264 for best compatibility
- Include multiple formats for broad support
- Consider poster images for large videos

### Audio

- MP3 is universally supported
- Use appropriate bitrate (128-192kbps for speech)

### Images

- Use WebP for better compression
- Provide appropriate sizes
- Use lazy loading (handled automatically)

## Troubleshooting

### Video Won't Play

1. Check file exists at the specified path
2. Verify file extension is recognized
3. Check browser console for codec errors
4. Try a different format (MP4/H.264 is safest)

### YouTube Not Embedding

1. Ensure URL is on its own line
2. Check URL format matches supported patterns
3. Verify video is publicly accessible

### OpenGraph Not Loading

1. Check URL is accessible from your network
2. Increase timeout: `--oembed-timeout-ms 2000`
3. Verify the target page has OpenGraph meta tags
