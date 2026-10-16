/**
 * Structure-preserving anonymization for `roundtrip-check` reproductions, so a
 * reader can share a failing case without sharing the note. Pure.
 *
 * Every letter becomes `x` (`X` when upper case), every digit `0`, combining
 * marks are dropped; everything else — whitespace, indentation, line breaks
 * (`\r` included) and every punctuation character markdown reads — is kept.
 * Letters stay letters and digits stay alphanumeric, so emphasis flanking,
 * escapes (`\a` is no escape either way) and autolink shapes parse as before.
 *
 * Kept verbatim because the parsers read the words themselves:
 *
 * - incomplete markers `TODO` `TK` `FIXME` `XXX`, `___Review History___`, and
 *   a flashcard history entry's date, time and rating (`- 2026-01-02 10:00 - Good`);
 * - alert tags (`[!NOTE]`), `@due(...)`/`@done(...)` with their dates, the
 *   `{{ vid(` shortcode name;
 * - an ordered list marker's number (`3.` starts a list at 3; `0.` would not);
 * - a fence's info word (```` ```chat ````), entity names (`&amp;`, `&#38;`),
 *   URL schemes (`https://`, `mailto:`), `www.`, `<![CDATA[`, and HTML tag
 *   names CommonMark gives meaning to (`<div>` starts an HTML block, `<xxx>`
 *   is a different kind);
 * - frontmatter `type:`/`style:` and their well-known values (`flashcard`, …).
 *
 * Link reference and footnote labels are renamed consistently and uniquely
 * (`[^1]` and `[^2]` must not both become `[^0]`, which would make the second
 * definition a duplicate): each defined label gets the next unused name of the
 * same shape, and every bracket naming it is rewritten to match.
 */

import { frontmatterCloseLine } from '../editor-frontmatter.js';

/** Words markdown or mbr reads, kept wherever they stand. */
const KEPT_WORD = /\b(?:TODO|TK|FIXME|XXX)\b/;

/** Frontmatter keys whose name and well-known values are kept. */
const KEPT_FRONTMATTER_KEYS = new Set(['type', 'style']);
const KEPT_FRONTMATTER_VALUES = new Set(['flashcard', 'person', 'organization', 'slides', 'outline', 'kanban']);

/**
 * HTML tag names kept: CommonMark's HTML block start conditions 1 and 6 (an
 * unknown name is condition 7, which behaves differently), plus common inline
 * elements.
 */
const HTML_TAGS = new Set(
  (
    'pre script style textarea address article aside base basefont blockquote body caption center col colgroup dd ' +
    'details dialog dir div dl dt fieldset figcaption figure footer form frame frameset h1 h2 h3 h4 h5 h6 head header ' +
    'hr html iframe legend li link main menu menuitem nav noframes ol optgroup option p param search section summary ' +
    'table tbody td tfoot th thead title tr track ul a abbr b br cite code del em i img ins kbd mark q s samp small ' +
    'span strong sub sup u var video audio source picture time input label button select font'
  ).split(' '),
);

/**
 * Inline tokens with a fixed meaning, tried in this order at each position:
 * a token matched here is kept (or, for tags and labels, partly kept).
 */
const INLINE_TOKEN = new RegExp(
  [
    /___Review History___/.source,
    /\[![A-Za-z]+\]/.source, // alert tag
    /@(?:due|done)\([^)\n]*\)/.source,
    /&(?:#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);/.source, // entity
    /\b[A-Za-z][A-Za-z0-9+.-]*:\/\//.source, // URL scheme
    /\bmailto:/.source,
    /\bwww\./.source,
    /<!\[CDATA\[/.source,
    /\{\{\s*vid\(/.source,
    KEPT_WORD.source,
    /<\/?(?![A-Za-z][A-Za-z0-9+.-]*:)[A-Za-z][A-Za-z0-9-]*/.source, // HTML tag name (not an autolink's scheme)
    /\[\^?[^[\]\n]+\]/.source, // bracketed text: maybe a label
  ].join('|'),
  'gu',
);

/** Letters → `x`/`X`, digits → `0`, combining marks dropped; nothing else changes. */
export function maskChars(text: string): string {
  return text.replace(/\p{M}|\p{L}|\p{N}/gu, (ch) => (/\p{M}/u.test(ch) ? '' : /\p{N}/u.test(ch) ? '0' : /[\p{Lu}\p{Lt}]/u.test(ch) ? 'X' : 'x'));
}

/** A label as CommonMark matches it: case-folded, whitespace collapsed. */
function labelKey(label: string): string {
  return label.trim().replace(/\s+/g, ' ').toLowerCase();
}

/** Link reference and footnote definitions: `[label]:` / `[^label]:` at a line start. */
const DEFINITION = /^ {0,3}(?:>\s*)*\[(\^?)((?:[^[\]\\\n]|\\.)+)\]:/gm;

/** Each defined label (by {@link labelKey}, caret included) → its new, unique name. */
function labelNames(text: string): Map<string, string> {
  const names = new Map<string, string>();
  const taken = new Set<string>();
  for (const match of text.matchAll(DEFINITION)) {
    const key = match[1] + labelKey(match[2]);
    if (names.has(key)) continue;
    const name = uniqueName(match[2], taken);
    taken.add(labelKey(name));
    names.set(key, name);
  }
  return names;
}

/**
 * `label` masked, or — when that name is taken — the same shape with a counter
 * written into its letters and digits from the end, so lengths, case and
 * punctuation are as authored and the name is still unique.
 */
function uniqueName(label: string, taken: ReadonlySet<string>): string {
  const masked = maskChars(label);
  if (!taken.has(labelKey(masked))) return masked;
  const slots = [...masked].map((ch, i) => (/[xX0]/.test(ch) ? i : -1)).filter((i) => i >= 0);
  for (let n = 1; ; n++) {
    const chars = [...masked];
    let rest = n;
    for (let s = slots.length - 1; s >= 0 && rest > 0; s--) {
      const i = slots[s];
      const digit = chars[i] === '0';
      const base = digit ? 10 : 26;
      const value = rest % base;
      rest = Math.floor(rest / base);
      const letter = LABEL_LETTERS[value];
      chars[i] = digit ? String(value) : chars[i] === 'X' ? letter.toUpperCase() : letter;
    }
    // More labels of this shape than its slots can count: lengthen it.
    const name = rest > 0 ? `${chars.join('')}${'x'.repeat(rest)}` : chars.join('');
    if (!taken.has(labelKey(name))) return name;
  }
}

/** Counter digits for renamed labels: `x` first, so counter 0 is the masked name itself. */
const LABEL_LETTERS = 'xyzabcdefghijklmnopqrstuvw';

/** Masks `text` around the tokens {@link INLINE_TOKEN} keeps. */
function maskInline(text: string, labels: ReadonlyMap<string, string>): string {
  let out = '';
  let at = 0;
  for (const match of text.matchAll(INLINE_TOKEN)) {
    const token = match[0];
    const start = match.index ?? 0;
    out += maskChars(text.slice(at, start)) + keepToken(token, labels);
    at = start + token.length;
  }
  return out + maskChars(text.slice(at));
}

/** What a matched token becomes. */
function keepToken(token: string, labels: ReadonlyMap<string, string>): string {
  if (token.startsWith('<') && !token.startsWith('<![')) {
    const slash = token.startsWith('</') ? '</' : '<';
    const name = token.slice(slash.length);
    return HTML_TAGS.has(name.toLowerCase()) ? token : slash + maskChars(name);
  }
  if (token.startsWith('[') && !token.startsWith('[!')) {
    const caret = token.startsWith('[^') ? '^' : '';
    const inner = token.slice(1 + caret.length, -1);
    const renamed = labels.get(caret + labelKey(inner));
    // Not a label: ordinary text in brackets (it may hold kept tokens itself).
    return renamed === undefined ? `[${caret}${maskInline(inner, new Map())}]` : `[${caret}${renamed}]`;
  }
  return token;
}

/**
 * A line's container markers — indentation, `>`, bullets, ordered markers
 * (their numbers kept), `: ` definitions — in any combination.
 */
const CONTAINERS = /^(?:[ \t]*(?:>|[*+-](?=[ \t]|$)|\d{1,9}[.)](?=[ \t]|$)|:(?=[ \t])))*[ \t]*/;

/** A task box after the markers (`[ ]`, `[x]`, `[-]`, `[>]`). */
const TASK_BOX = /^\[[ xX>-]\](?=[ \t]|$)[ \t]*/;

/** What may follow the markers and is kept: a flashcard history entry, or a fence opener and its info word. */
const KEPT_LEAD = new RegExp(
  [
    /\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}(?::\d{2})?)?\s+-\s+(?:Again|Hard|Good|Easy|Fail)\b/.source,
    /(?:`{3,}|~{3,})[ \t]*[^\s`{]*/.source,
  ]
    .map((alt) => `^(?:${alt})`)
    .join('|'),
);

/** One body line: its markers, task box and kept lead verbatim, the rest masked. */
function maskBodyLine(line: string, labels: ReadonlyMap<string, string>): string {
  let prefix = CONTAINERS.exec(line)?.[0] ?? '';
  for (const kept of [TASK_BOX, KEPT_LEAD]) prefix += kept.exec(line.slice(prefix.length))?.[0] ?? '';
  return prefix + maskInline(line.slice(prefix.length), labels);
}

/** One frontmatter line: `key: value`, keeping the keys and values mbr reads. */
function maskFrontmatterLine(line: string, labels: ReadonlyMap<string, string>): string {
  const field = /^(\s*(?:-\s+)?)([A-Za-z_][\w-]*)(\s*:)(.*)$/s.exec(line);
  if (!field) return maskInline(line, labels);
  const [, indent, key, colon, value] = field;
  if (!KEPT_FRONTMATTER_KEYS.has(key)) return indent + maskChars(key) + colon + maskInline(value, labels);
  const bare = value.trim().replace(/^["']|["']$/g, '');
  return indent + key + colon + (KEPT_FRONTMATTER_VALUES.has(bare) ? value : maskInline(value, labels));
}

/** Anonymizes a markdown document (see the module docs). */
export function anonymize(markdown: string): string {
  const labels = labelNames(markdown);
  const lines = markdown.split('\n');
  // The editor's frontmatter rule (`frontmatterCloseLine`), after a BOM.
  const fmEnd = frontmatterCloseLine([lines[0]?.replace(/^\uFEFF/, '') ?? '', ...lines.slice(1)]);
  return lines
    .map((line, i) => {
      if (fmEnd > 0 && i <= fmEnd) return i === 0 || i === fmEnd ? line : maskFrontmatterLine(line, labels);
      return maskBodyLine(line, labels);
    })
    .join('\n');
}
