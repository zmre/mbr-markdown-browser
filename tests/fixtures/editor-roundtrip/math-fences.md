# Math fences

A fenced block keeps multi-line math together, where `$$` would not.

```latex
\begin{aligned}
a &= b \\
- c &= d

> e &= f
\end{aligned}
```

```katex
x^2 + y^2 = z^2
```

```math
\begin{cases}
1 & x > 0 \\

0 & \text{otherwise}
\end{cases}
```

```asciimath
sum_(i=1)^n i^3=((n(n+1))/2)^2

- x
```

```tex
\documentclass{article}
- not math, just TeX source

\begin{document}
\end{document}
```

- In a list:

  ```LaTeX
  E = mc^2
  ```
