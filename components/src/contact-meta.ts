/**
 * Which frontmatter keys the info panel's Metadata table leaves to the contact
 * card on `type: person` / `type: organization` pages.
 *
 * The card (`templates/_contact_card.html`) already shows these, formatted;
 * the panel would otherwise repeat them raw and flattened (`Emails.work`,
 * `Dates.anniversary --06-10`). `dates.*` is hidden rather than re-humanized
 * here: the card renders them through Rust's `PartialDate`, and a second date
 * formatter in the main bundle would be one more copy to drift.
 *
 * The type test is the exact match the templates and `src/contact.rs` use.
 */
const CARD_ROOTS = new Set([
  'emails',
  'phones',
  'urls',
  'social',
  'im',
  'addresses',
  'dates',
  'first_name',
  'middle_name',
  'last_name',
  'prefix',
  'suffix',
  'company',
  'department',
  'job_title',
])

export function isContactType(type: unknown): boolean {
  return type === 'person' || type === 'organization'
}

/** True when `key` belongs to the card on a page of this `type`. */
export function isCardKey(type: unknown, key: string): boolean {
  if (!isContactType(type)) return false
  const dot = key.indexOf('.')
  return CARD_ROOTS.has(dot === -1 ? key : key.slice(0, dot))
}
