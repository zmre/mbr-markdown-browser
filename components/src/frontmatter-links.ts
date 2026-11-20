/**
 * Frontmatter values that are links (`customer: "[[Customer X]]"`,
 * `contracts: ["/c/msa.pdf"]`), as `site.json` publishes them per note in
 * `frontmatter_links` (crates/mbr-core/src/frontmatter_links.rs,
 * `links_by_key`): `{key: entry}` for a scalar value, `{key: [entries]}` for a
 * list (only its link elements, in list order).
 *
 * Pure: the info panel renders the parts this returns.
 */

/** One link as `site.json` publishes it. */
export interface FrontmatterLinkEntry {
  /** Site-absolute target, also when nothing is there. */
  url: string;
  /** The target's title (note title, file name), else the value's own name. */
  title: string;
  /** Label if the wikilink had one, else `title`. */
  text: string;
  /** The value as authored, trimmed. */
  raw: string;
  /** Whether the target exists. */
  resolved: boolean;
}

export type FrontmatterLinks = Record<string, FrontmatterLinkEntry | FrontmatterLinkEntry[]>;

/** A piece of a metadata value: plain text, or a link to a resolved target. */
export type ValuePart = { text: string; url?: string };

/** The `frontmatter_links` of a `site.json` entry, or `{}` when it has none. */
export function frontmatterLinksOf(entry: unknown): FrontmatterLinks {
  const links = (entry as { frontmatter_links?: unknown } | null)?.frontmatter_links;
  return links && typeof links === 'object' && !Array.isArray(links)
    ? (links as FrontmatterLinks)
    : {};
}

/**
 * Splits a frontmatter `value` into parts, linking the elements that are
 * resolved frontmatter links. `null` when `key` has no links, so the caller
 * keeps its own formatting. An unresolved link stays text (as authored), like
 * an unresolved relationship neighbour.
 */
export function linkedValueParts(
  value: unknown,
  links: FrontmatterLinks,
  key: string,
): ValuePart[] | null {
  const forKey = links[key];
  if (!forKey) return null;
  const entries = Array.isArray(forKey) ? forKey : [forKey];
  const part = (raw: string): ValuePart => {
    const entry = entries.find((e) => e.raw === raw.trim());
    return entry?.resolved ? { text: entry.text, url: entry.url } : { text: raw };
  };
  if (Array.isArray(value)) return value.map((v) => part(String(v)));
  return [part(String(value))];
}
