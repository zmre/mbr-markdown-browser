# Developing MBR

## Quick Start

For rapid UI iteration without full Rust rebuilds, use the `--template-folder` flag to load templates and assets from disk instead of the compiled-in defaults.

### Terminal 1: Component Watcher

Watches TypeScript sources and rebuilds to `templates/components-js/` on change:

```bash
cd components
bun install     # First time only
bun run watch
```

### Terminal 2: Rust Server with Hot Reload

Watches Rust files and restarts the server, while ignoring template/component changes (those are handled by Terminal 1):

```bash
cargo watch -i "templates/**" -i "components/**" -i "*.md" -q -c -x 'run --release -- -s --template-folder ./templates README.md'
```

This command:
- `-i "templates/**"` - Ignores template file changes (HTML, CSS, JS)
- `-i "components/**"` - Ignores TypeScript source changes
- `-i "*.md` - Ignores the markdown files we might be using in this dir for testing
- `-q` - Quiet mode (less cargo-watch output)
- `-c` - Clears screen between runs
- `--template-folder ./templates` - Loads templates from disk instead of compiled defaults

### How It Works

With `--template-folder ./templates`:

1. **Templates** (`*.html`) are loaded from `./templates/` with fallback to compiled defaults
2. **Assets** (`*.css`, `*.js`) are served from `./templates/` with fallback to compiled defaults
3. **Components** (`/.mbr/components/*`) are mapped to `./templates/components-js/*`
4. **File watcher** monitors both the markdown directory and the template folder for hot reload

When you edit:
- **Rust files** → cargo watch rebuilds and restarts the server
- **HTML/CSS files in templates/** → Browser auto-reloads via WebSocket
- **TypeScript in components/src/** → Vite rebuilds to `templates/components-js/`, then browser auto-reloads

## Build Commands

```bash
# Build release binary
cargo build --release

# Run tests
cargo test

# Build components only
cd components && bun run build

# Check for issues
cargo clippy
```

## Architecture Notes

The `--template-folder` flag serves dual purposes:

1. **Development**: Point to `./templates` for rapid UI iteration
2. **User customization**: Share a custom theme across multiple markdown repos

```bash
# Use a shared theme for any markdown repo
mbr -s --template-folder ~/my-mbr-theme /path/to/markdown/repo
```

### Fallback Chain

Asset resolution follows this priority:
1. `--template-folder` path (if specified)
2. `.mbr/` folder in the markdown repo
3. Compiled-in defaults

This means you can partially override - missing files fall back to defaults.
