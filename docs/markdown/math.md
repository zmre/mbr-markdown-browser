---
title: Math
description: "LaTeX math with KaTeX: inline, display, math fences and AsciiMath"
order: 8
---

# Math

mbr supports mathematical notation using [KaTeX](https://katex.org/), rendered from LaTeX syntax.

## Inline Math

Wrap expressions in single dollar signs for inline math:

```markdown
The quadratic formula is $x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$ which solves $ax^2 + bx + c = 0$.
```

Renders as: The quadratic formula is $x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$ which solves $ax^2 + bx + c = 0$.

## Display Math

Use double dollar signs for block-level equations:

```markdown
$$
\int_{-\infty}^{\infty} e^{-x^2} dx = \sqrt{\pi}
$$
```

Renders as:

$$
\int_{-\infty}^{\infty} e^{-x^2} dx = \sqrt{\pi}
$$

## More Examples

**Matrices:**

$$
\begin{pmatrix}
a & b \\
c & d
\end{pmatrix}
\begin{pmatrix}
x \\
y
\end{pmatrix} =
\begin{pmatrix}
ax + by \\
cx + dy
\end{pmatrix}
$$

**Summations and products:**

$$
\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}
$$

## Multi-line math with fenced blocks

`$$ … $$` is *inline* markdown: the math is a span inside a paragraph, so
anything that ends the paragraph ends the math too. A blank line does, and so
does a line that starts a list, quote or heading (`- `, `+ `, `* `, `1. `,
`> `, `# `), which is easy to write in an `aligned` environment:

```markdown
$$
\begin{aligned}
a &= b \\
- c &= d
\end{aligned}
$$
```

Here `- c &= d` starts a bullet list, and the reader gets a broken paragraph
and a list instead of an equation.

A fenced code block whose language is `latex`, `katex` or `math` is rendered as
display math instead. A fence's body is taken verbatim, so blank lines and
lines starting with `-` or `>` are just TeX:

````markdown
```latex
\begin{aligned}
a &= b \\
- c &= d
\end{aligned}
```
````

Renders as:

```latex
\begin{aligned}
a &= b \\
- c &= d
\end{aligned}
```

The language is the first word of the info string and is matched without
regard to case (` ```LaTeX ` works). Use ` ```tex ` (or any other language)
when you want TeX *source* shown as a highlighted code block rather than
typeset. In QuickLook previews, which do not load KaTeX, math fences are shown
as highlighted code.

## AsciiMath

A ` ```asciimath ` fence holds [AsciiMath](https://asciimath.org/), a lighter
notation that mbr converts to TeX in the browser (with
[asciimath2tex](https://github.com/christianp/asciimath2tex)) and then renders
with KaTeX. The converter is loaded only on pages that have an AsciiMath block.

````markdown
```asciimath
sum_(i=1)^n i^3 = ((n(n+1))/2)^2
```
````

Renders as:

```asciimath
sum_(i=1)^n i^3 = ((n(n+1))/2)^2
```

Each fence is one expression: a line break is treated as a space, as in other
AsciiMath renderers. Use one fence per equation. AsciiMath is available only in
fences, not inline.

See the [KaTeX documentation](https://katex.org/docs/supported.html) for the full list of supported LaTeX commands.
