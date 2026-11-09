---
title: Code Blocks
description: Fenced code blocks with syntax highlighting
order: 6
---

# Code Blocks

Code blocks are highlighted using highlight.js:

## Live Examples

```rust
fn main() {
    let message = "Hello from mbr!";
    println!("{}", message);
}
```

```python
def greet(name: str) -> str:
    """Return a friendly greeting."""
    return f"Hello, {name}!"
```

```javascript
const render = async (markdown) => {
  const html = await mbr.parse(markdown);
  document.body.innerHTML = html;
};
```

## Supported Languages

`bash`, `javascript`, `typescript`, `python`, `ruby`, `rust`, `go`, `java`, `json`, `yaml`, `toml`, `html`, `css`, `sql`, `markdown`, and many more.
