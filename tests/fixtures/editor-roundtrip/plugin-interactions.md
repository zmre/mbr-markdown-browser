# Plugin interactions &amp; $x$ [[Note]] {#interactions .c}

Every editor plugin on one page, where their syntax meets: wikilinks beside
reference links [[Note]] and [the docs][r], [[Other|alias]][r], [[a]][[b]],
[x][] and [y]; `\$` beside math: \$5, $x^2$, \$10, $a\,b$, [[Cost $5]],
$[[x]]$ and $$\{a\}$$; entities inside links and aliases:
[[Note#Head|al &amp; ias]], [&amp;][r], [$x$][r], [[$y$]].

Escaped brackets stay text: \[not a link], \&amp; and \[!NOTE].

> [!NOTE]
> Body &amp; more [x][r], [[Note]], $a\,b$ and \$3.

> [!TIP] [[Note]]
> An alert whose first line carries a wikilink.

* [-] cancelled [r] &amp; $x$
* [>] moved [[Note]]
* [x] done
* [ ] [link][r] and [[Note]]

A defined `[!warning]` label leaves the alert an alert; a defined `[-]`
label makes `[-]` a reference link and the item a plain one, to the
renderer and the editor alike:

> [!WARNING]
> Still an alert.

* [-] still cancelled

An alert whose body is a definition list, and list items that start with
one:

> [!TIP]
> Term in an alert
> : its definition

> [!NOTE]
> : a definition with no term is a paragraph

* Term in a list item
  : its definition
* [ ] Task term
  : task definition

![[embed.png]], ![alt](a.png) and ![ref][img].

--- {#s .c}

After &amp; [[x]].

[r]: https://example.com
[x]: /x
[y]: /y
[img]: /i.png
[!warning]: /w
[-]: /dash
