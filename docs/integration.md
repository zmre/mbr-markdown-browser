---
title: Integration
description: Using mbr with other tools
---

# Integration with Other Tools

mbr works seamlessly with existing markdown repositories and can integrate into various workflows.

## Obsidian Vaults

mbr can serve Obsidian vaults without modification:

```bash
mbr -s ~/Documents/ObsidianVault
```

### Compatibility

| Feature | Status | Notes |
|---------|--------|-------|
| Standard markdown | Full | All basic syntax works |
| YAML frontmatter | Full | Tags, aliases, etc. |
| Wikilinks | Partial | Standard `[[link]]` renders as-is |
| Embedded files | Partial | Images work, transclusion doesn't |
| Callouts | Full | GitHub-style alerts are similar |
| Mermaid | Full | Same syntax |

### Configuration for Obsidian

```toml
# .mbr/config.toml
index_file = "index.md"
ignore_dirs = [".obsidian", ".trash"]
static_folder = "attachments"
```

### Wiki Links

mbr doesn't convert `[[wikilinks]]` to HTML links automatically. For full wikilink support, consider preprocessing or using standard markdown links.

## VS Code

### Live Preview Workflow

1. Start mbr server:
   ```bash
   mbr -s ~/notes
   ```

2. Split your VS Code window

3. Open browser in one pane, editor in another

4. Edit and save - browser refreshes automatically

### Tasks Integration

Add to `.vscode/tasks.json`:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "mbr serve",
      "type": "shell",
      "command": "mbr -s .",
      "isBackground": true,
      "problemMatcher": []
    },
    {
      "label": "mbr build",
      "type": "shell",
      "command": "mbr -b --output ./build .",
      "group": "build"
    }
  ]
}
```

## Git Integration

### Committing `.mbr/` Folder

Whether to commit `.mbr/` depends on your use case:

**Commit if:**
- You want shared theming across clones
- Configuration is project-specific
- Custom templates are part of the project

**Don't commit if:**
- Using personal preferences
- Different team members want different themes
- Repository is for content only

### `.gitignore` Suggestions

```gitignore
# mbr build output
build/

# Personal mbr preferences (if not sharing)
.mbr/user.css
.mbr/config.toml

# Keep shared templates
!.mbr/index.html
!.mbr/theme.css
```

## CI/CD Deployment

### GitHub Actions

Complete workflow for building and deploying docs:

```yaml
# .github/workflows/docs.yml
name: Documentation

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages-${{ github.ref }}
  cancel-in-progress: true

jobs:
  build-docs:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: DeterminateSystems/determinate-nix-action@main

      - uses: cachix/cachix-action@v16
        with:
          name: your-cache
          authToken: ${{ secrets.CACHIX_AUTH_TOKEN }}

      - name: Build mbr
        run: nix build -L .#mbr

      - name: Build documentation
        run: ./result/bin/mbr -b --output ./docs-build ./docs

      - name: Upload artifact
        uses: actions/upload-pages-artifact@v3
        with:
          path: ./docs-build

  deploy:
    if: github.ref == 'refs/heads/main'
    needs: build-docs
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - name: Deploy to GitHub Pages
        id: deployment
        uses: actions/deploy-pages@v4
```

### Netlify

```toml
# netlify.toml
[build]
  publish = "build"
  command = "nix run github:zmre/mbr -- -b ."

[build.environment]
  NODE_VERSION = "20"
```

### Cloudflare Pages

```yaml
# In Cloudflare dashboard:
# Build command: nix run github:zmre/mbr -- -b --output ./build .
# Build output: /build
```

## Other Static Site Generators

### Comparison

| Feature | mbr | Zola | Hugo | mdBook |
|---------|-----|------|------|--------|
| Zero config | Yes | No | No | No |
| Any structure | Yes | No | No | No |
| Live preview | Yes | Yes | Yes | Yes |
| Search | Yes | Yes | Yes | Yes |
| Themes | Yes | Yes | Yes | Yes |
| Wikilinks | No | No | No | No |
| Build speed | Fast | Fast | Very Fast | Fast |

### Migration Notes

**From Zola/Hugo:**
- Remove frontmatter `template` fields (mbr auto-detects)
- Convert shortcodes to mbr syntax or standard markdown
- Move assets to `static/` folder

**From mdBook:**
- Remove `SUMMARY.md` (mbr generates navigation)
- Flatten nested structure if desired
- Update internal links

**To Static Generator:**
- mbr output is plain HTML, compatible with any host
- Search uses Pagefind (works standalone)
- No runtime dependencies

## IDE/Editor Integration

### Vim/Neovim

Auto-preview on open and updates on save:

```vim
" Start mbr gui for current file and current repo
" switch "-g" for "-s" to start a server and use your browser instead
autocmd BufEnter *.md silent! !pgrep -x mbr || mbr -g %:p:h &
```

Or in neovim only, using lua config, start a background job on a shortcut key:

```lua
vim.keymap.set(
  "n", -- normal mode
  "<leader>m", -- leader key (like space or comma) followed by m
  function()
     vim.fn.jobstart({'mbr', vim.fn.expand('%:p')}, {detach = true}) -- background the previewer
  end,
  { silent = true, noremap = true }
)
```

### Emacs

```elisp
;; Start mbr server for markdown mode
(defun start-mbr-server ()
  (when (eq major-mode 'markdown-mode)
    (start-process "mbr" nil "mbr" "-s" default-directory)))

(add-hook 'markdown-mode-hook 'start-mbr-server)
```

## Browser Extensions

mbr works with standard browser extensions:

- **Dark Reader**: Compatible (respects CSS variables)
- **Vimium**: Compatible (standard HTML navigation)
- **uBlock Origin**: No issues (no ads/tracking)

## API Usage

### Site Metadata

Fetch all site data programmatically:

```javascript
const response = await fetch('/.mbr/site.json');
const data = await response.json();

// data.files - array of all markdown files with metadata
// data.folders - directory structure
```

### Search API

Integrate search into external tools:

```javascript
const results = await fetch('/.mbr/search', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    q: 'search query',
    limit: 10,
    scope: 'content'
  })
}).then(r => r.json());
```

## Troubleshooting Integration

### Port Conflicts

If mbr's default port (5200) conflicts:

```bash
MBR_PORT=3000 mbr -s ~/notes
```

### Path Resolution

When integrating with tools that use different path conventions:

```toml
# .mbr/config.toml
static_folder = "assets"      # Match your tool's convention
index_file = "README.md"       # Or "index.md"
```

### Asset Handling

For tools that expect assets in specific locations:

```bash
# Symlink your assets folder
ln -s /path/to/actual/assets ./static
```
