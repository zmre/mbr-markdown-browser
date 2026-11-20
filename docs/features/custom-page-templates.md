---
title: Custom Page Templates
description: Give one note a layout of its own with template in its frontmatter — invoices, letters, CVs
order: 17
---

# Custom Page Templates

**Works in:** GUI · Server · Static site · iOS · QuickLook

A note can name its own page layout. Add `template:` to its
[frontmatter](../markdown/frontmatter/) and mbr renders the note through that
file instead of its usual page (`index.html`):

```yaml
---
title: Invoice 2026-001
template: invoice.html
---
```

The template is an ordinary [Tera](https://keats.github.io/tera/docs/) file,
the same language as mbr's own [templates](../customization/templates/). It
gets everything a normal page gets: the rendered body as `markdown`, every
frontmatter field, and the whole frontmatter, structure intact, as
`frontmatter`. That makes a note a good place to keep a document whose details
are data: an invoice with line items, a letter with an address block, a CV.

## Where the template lives

`template` is a path to an `.html` file, **relative to the note's folder**:

| Value | Finds |
|-------|-------|
| `invoice.html` | beside the note |
| `../layouts/invoice.html` | in a sibling folder |
| `/layouts/invoice.html` | from the repository root (leading `/`) |

The file must be inside the repository. mbr refuses a path that climbs above
the root, a symlink that leads out of it (a read-only
[symlink mount](../reference/configuration/) counts as inside), anything in a
hidden folder (`.mbr/`, `.git/`) and anything that is not an `.html` file.

Keep templates out of a folder named `templates/`: it is in the default
`ignore_dirs`, so the page still renders, but the watcher never sees you save
the file, and the page will not live-reload.

## A worked example: an invoice

```text
clients/
  acme/
    2026-001.md
    invoice.html
    lines.html
images/
  logo.png
```

`clients/acme/2026-001.md` holds the data and a note to the client:

```markdown
---
title: Invoice 2026-001
template: invoice.html
date: 2026-11-17
client:
  name: Acme Corp
  address: [1 Main St, Springfield]
tax_rate: 0.2
items:
  - {desc: Design, qty: 12, rate: 95}
  - {desc: Development, qty: 30, rate: 110.5}
  - {desc: Hosting (annual), qty: 1, rate: 240}
---

Payment is due within 30 days. Thank you for your business.
```

`clients/acme/invoice.html` lays it out:

```html
<!doctype html>
<html lang="en">
<head>
  {% include "/.mbr/_head.html" %}
  {% include "/.mbr/_export_ready.html" %}
  <title>{{ title }}</title>
  <style>@page { size: A4; margin: 18mm; }</style>
</head>
<body>
  <main class="container">
    <img src="{{ url(path='/images/logo.png') }}" alt="Logo" width="120">
    <h1>{{ title }}</h1>
    <p>{{ date }}</p>
    <address>
      {{ frontmatter.client.name }}<br>
      {% for line in frontmatter.client.address %}{{ line }}<br>{% endfor %}
    </address>

    {% include "lines.html" %}

    {{ markdown | safe }}
  </main>
</body>
</html>
```

`clients/acme/lines.html` is a partial beside it, which does the arithmetic:

```html
{% set_global subtotal = 0 %}
<table>
  <thead><tr><th>Item</th><th>Qty</th><th>Rate</th><th>Amount</th></tr></thead>
  <tbody>
  {% for item in frontmatter.items %}
    {% set amount = item.qty * item.rate %}
    {% set_global subtotal = subtotal + amount %}
    <tr>
      <td>{{ item.desc }}</td>
      <td>{{ item.qty }}</td>
      <td>{{ item.rate | round(precision=2) }}</td>
      <td>{{ amount | round(precision=2) }}</td>
    </tr>
  {% endfor %}
  </tbody>
  <tfoot>
    {% set tax = subtotal * frontmatter.tax_rate %}
    {% set total = subtotal + tax %}
    <tr><td colspan="3">Subtotal</td><td>{{ subtotal | round(precision=2) }}</td></tr>
    <tr><td colspan="3">Tax</td><td>{{ tax | round(precision=2) }}</td></tr>
    <tr><th colspan="3">Total</th><th>{{ total | round(precision=2) }}</th></tr>
  </tfoot>
</table>
```

Open `/clients/acme/2026-001/` and the invoice is the page. The template files
themselves are not served or published (see below). `_export_ready.html` and
the `@page` rule are for [saving it as a PDF](#printing-and-pdf-export).

## What a template can use

### Variables

A template gets exactly the context `index.html` gets (see
[Template Variables](../customization/templates/#markdown-pages--index-html)),
plus:

| Variable | What it is |
|----------|------------|
| `frontmatter` | The note's whole frontmatter as written: nested maps stay maps, lists of maps stay lists, numbers stay numbers (`{{ item.qty * item.rate }}` works), `null` stays `null` |
| `markdown` | The rendered body (write it with `| safe`) |
| `title`, `date`, … | Each frontmatter field as its own variable, as on every page. Nested maps appear flattened (`client.name`) and floats as text here; use `frontmatter.…` for the real values |

`frontmatter` is on every markdown page, so a customized `.mbr/index.html` can
use it too. A frontmatter field that is itself called `frontmatter` does not
replace it; it is `frontmatter.frontmatter`.

### Including mbr's templates and your own

- **mbr's templates** are available as `/.mbr/<name>`:
  `{% include "/.mbr/_head.html" %}` brings in mbr's stylesheets and scripts,
  `{% include "/.mbr/_nav.html" %}` the header. They are your `.mbr/`
  overrides where you have them, mbr's built-in ones otherwise, and they
  include each other exactly as they do on any page.
- **Your own partials**: every `.html` file directly in the template's folder
  can be included or extended by its file name — `{% include "lines.html" %}`,
  `{% extends "base.html" %}`. Subfolders are not read.
- A file beside the template that has **the same name as one of mbr's**
  (`_head.html`, `index.html`, …) is never used under that name: mbr's partials
  always include mbr's partials. Rename it.
- A partial that does not parse is skipped. If the template uses it, the error
  names it; if not, it does no harm.

A template that does not include `/.mbr/_head.html` and `/.mbr/_footer.html`
gets none of mbr's interface: no sidebar, no search, no live reload. That is
often the point (an invoice to print), but include them if you want the page
to behave like the rest of the site. mbr's scripts themselves are
`/.mbr/_scripts.html`, which needs `_head.html` before it.

### URLs

Write a URL to a file in the repository relative to the **repository root**,
with the `url()` helper:

```html
<img src="{{ url(path='/images/logo.png') }}">
```

It writes `/images/logo.png` on the server, in the GUI and in QuickLook, and a
path relative to the page in a static build (`../../../images/logo.png`), so a
site published under a sub-path still works. It is the same as the idiom mbr's
own templates use, which also works:

```html
<img src="{{ relative_root | default(value='/') | safe }}images/logo.png">
```

`url()` escapes its result for HTML once; `{{ url(…) | safe }}` is the same.
A URL with a scheme or a host (`https://…`, `//cdn…`, `mailto:`) passes through
unchanged. URLs inside the template are not otherwise rewritten, and a
relative URL (`logo.png`) would be relative to the page's own address
(`/clients/acme/2026-001/`), not to the template's folder.

## Printing and PDF export

A template page can be saved as a PDF like any other (`mbr --pdf`, **Save as
PDF…**), with two things to know:

- **Readiness.** The export waits for the page to say it has finished
  rendering, and the script that says so is part of mbr's scripts
  (`/.mbr/_scripts.html`). A template without them should include
  `{% include "/.mbr/_export_ready.html" %}` in its `<head>`: a small script
  that does nothing except during an export, when it waits for images, fonts
  and (if the template loads mermaid) diagrams and then signals ready.
  Without either, every export waits out the whole timeout (30 seconds by
  default). For a page that can run no script of mbr's at all, `ready = "load"`
  in `[pdf]` prints on the `load` event instead. See
  [PDF Export](pdf-export/#pages-with-a-custom-template).
- **Paper and margins.** The template's own `@page` rule wins over the
  `[pdf]` paper and margin: `@page { size: A5; margin: 20mm }` gives an A5 PDF
  with 20 mm margins whatever `[pdf]` says, and sides or a size it leaves out
  come from `[pdf]`. See
  [Paper and margins set by the page](pdf-export/#paper-and-margins-set-by-the-page).

A host application that links mbr as a library exports the same way; see
[Exporting PDFs from a host application](pdf-export/#exporting-pdfs-from-a-host-application).

## Template files are not published

A file that some note names as its template is page structure, not content:

- the server answers **404** for it (`/clients/acme/invoice.html`);
- a static build does not copy it;
- it is left out of `media.json`, the media browser and search.

This applies only to the file a note names. Its partials (`lines.html`) are
ordinary files and are published like any other `.html` in the repository.

## When something goes wrong

A missing, misplaced or broken template is reported, never quietly replaced by
the normal page — for an invoice, the normal page would be the wrong document.

| Mode | What happens |
|------|--------------|
| Server and GUI | The page is mbr's error page (status 500) naming the note, the template and the problem, e.g. ``invoices/2026-001.md: template `invoice.html` failed to render: … Variable `client.nme` not found``. Fix the file and reload |
| iOS (a cloud vault) | A template, or a partial it includes, that is not on the device yet is downloaded first: the page waits a few seconds for it, then shows the usual "Downloading…" page, which retries by itself until it arrives. Partials the template never uses are not downloaded |
| Static build (`mbr -b`) | The build **fails** with a non-zero exit after trying every page, listing each note, its template and the problem |
| `mbr -o` (stdout) | Exits with the error |
| `mbr --pdf` | Refuses before starting (exit status 2), naming the note, the template and the problem, and writes no PDF — it never prints the error page. Save as PDF… in the GUI prints the page on screen, which is then the error page |
| QuickLook | Shows the normal preview; the problem goes to the system log |

## Live reload

Saving the template, or any `.html` file beside it, reloads every open page
rendered with it, just as saving the note does.

## In QuickLook

A QuickLook preview of the note renders the template too, with the same
context (no sidebar data or live features). Images and other local files it
references are attached the way a preview attaches a note's images, and
mbr's stylesheets (`/.mbr/pico.min.css`, `/.mbr/theme.css`, `/.mbr/user.css`,
or any `.css` in your `.mbr/`) come from your `.mbr/` folder or mbr's built-in
copies. Scripts do not run there. Thumbnails keep showing the ordinary page.
