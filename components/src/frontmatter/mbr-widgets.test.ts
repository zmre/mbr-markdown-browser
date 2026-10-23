import { afterEach, describe, expect, it } from 'vitest';
import type { MbrFmEditor } from './properties-form.js';
import { builtinCatalog, addableFields } from './fields.js';
import { inferCatalog } from './infer.js';
import { toValue } from './model.js';
import { MBR_WIDGETS } from './mbr-widgets.js';
import { DEFAULT_RELATIONSHIP_TYPES, createActorMemory } from './mbr-logic.js';
import type { MbrFmCombobox } from './fm-combobox.js';
import { commit, mount, row, settle, type, value } from './test-support.js';

/** Mounts the form with mbr's widgets and the default relationship registry. */
async function mountMbr(text: string, setup: (el: MbrFmEditor) => void = () => {}) {
  return mount(text, (el) => {
    el.widgets = MBR_WIDGETS;
    el.catalog = builtinCatalog({ relationshipTypes: DEFAULT_RELATIONSHIP_TYPES });
    setup(el);
  });
}

const combo = (root: Element, selector = 'mbr-fm-combobox'): MbrFmCombobox => root.querySelector<MbrFmCombobox>(selector)!;

function button(root: Element, text: string): HTMLButtonElement {
  const found = [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text);
  if (!found) throw new Error(`no button “${text}”`);
  return found;
}

function edge(el: MbrFmEditor, i: number): HTMLElement {
  return value(el, ['relationships']).querySelector<HTMLElement>(`li.fm-edge[data-index="${i}"]`)!;
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('type', () => {
  it('suggests mbr’s types with their help, then the vault’s by use, and accepts anything', async () => {
    const inferred = inferCatalog([{ frontmatter: { type: 'Metric' } }, { frontmatter: { type: 'Metric' } }, { frontmatter: { type: 'Playbook' } }]);
    const { el } = await mountMbr('type: note\n', (e) => {
      e.inferred = inferred;
    });
    const c = combo(value(el, ['type']));
    expect(c.options.map((o) => o.value)).toEqual(['slides', 'flashcard', 'person', 'organization', 'Metric', 'Playbook']);
    expect(c.options[2].detail).toMatch(/contact/i);
    expect(c.openOnFocus).toBe(true);
    commit(c, 'BigQuery Table');
    await settle(el);
    expect(el.text).toBe('type: BigQuery Table\n');
    expect(el.querySelector('.fm-hint')).toBeNull();
  });

  it('shows contact and Attested Computation fields only for their types', async () => {
    const text = 'type: person\nemails:\n  work: a@x\nruntime: bigquery\n';
    const { el } = await mountMbr(text);
    expect(value(el, ['emails']).querySelector('ol.fm-labeled')).not.toBeNull();
    expect(value(el, ['runtime']).querySelector('mbr-fm-combobox')).toBeNull();
    commit(combo(value(el, ['type'])), 'Attested Computation');
    await settle(el);
    // Not a contact any more: emails is an ordinary group; runtime now has its suggestions.
    expect(value(el, ['emails']).querySelector('ol.fm-labeled')).toBeNull();
    expect(combo(value(el, ['runtime'])).options.map((o) => o.value)).toContain('bigquery');
    const fm = toValue(el.model) as Record<string, unknown>;
    expect(addableFields(fm, el.catalog).map((f) => f.key)).toContain('parameters');
    expect(addableFields({ type: 'note' }, el.catalog).map((f) => f.key)).not.toContain('parameters');
  });

  it('orders "+ Add property": mbr’s own, then the type’s, then OKF, then the vault’s', async () => {
    const inferred = inferCatalog([{ frontmatter: { project: 'x' } }]);
    const keys = addableFields({ type: 'person' }, builtinCatalog(), inferred).map((f) => f.key);
    expect(keys.indexOf('title')).toBeLessThan(keys.indexOf('first_name'));
    expect(keys.indexOf('first_name')).toBeLessThan(keys.indexOf('status'));
    expect(keys.indexOf('status')).toBeLessThan(keys.indexOf('project'));
  });
});

describe('tags', () => {
  it('keeps a comma string a string', async () => {
    const { el } = await mountMbr('tags: rust, web\n');
    expect([...value(el, ['tags']).querySelectorAll('.fm-chip > span')].map((s) => s.textContent)).toEqual(['rust', 'web']);
    commit(combo(value(el, ['tags'])), 'cli');
    await settle(el);
    expect(el.text).toBe('tags: rust, web, cli\n');
    value(el, ['tags']).querySelector<HTMLButtonElement>('[aria-label="Remove rust"]')!.click();
    await settle(el);
    expect(el.text).toBe('tags: web, cli\n');
  });

  it('suggests tags from every tag source, nested ones included', async () => {
    const inferred = inferCatalog([{ frontmatter: { tags: ['rust'] } }, { frontmatter: { taxonomy: { performers: ['Miles'] } } }], {
      tagFields: ['tags', 'taxonomy.performers'],
    });
    const { el } = await mountMbr('taxonomy:\n  performers:\n    - Coltrane\n', (e) => {
      e.catalog = builtinCatalog({ tagSources: [{ field: 'tags' }, { field: 'taxonomy.performers' }] });
      e.inferred = inferred;
    });
    const c = combo(value(el, ['taxonomy', 'performers']));
    expect(c.options.map((o) => o.value)).toEqual(['Miles', 'rust']);
  });
});

describe('note link', () => {
  it('suggests notes and writes a quoted [[Title]]', async () => {
    const { el } = await mountMbr('type: person\ncompany:\n', (e) => {
      e.siteFiles = [{ url_path: '/orgs/acme/', frontmatter: { title: 'Acme Corp' } }];
    });
    const c = combo(value(el, ['company']));
    expect(c.options).toEqual([{ value: '[[Acme Corp]]', label: 'Acme Corp', detail: '/orgs/acme/' }]);
    commit(c, '[[Acme Corp]]', 'pick');
    await settle(el);
    expect(el.text).toBe('type: person\ncompany: "[[Acme Corp]]"\n');
  });
});

describe('relationships', () => {
  const text = 'title: Jane\nrelationships:\n  - type: spouse\n    to: "[[Mary]]"\n    married: 1948-06-01\n';

  it('draws each edge with its sentence, and other keys as rows', async () => {
    const { el } = await mountMbr(text);
    expect(edge(el, 0).querySelector('.fm-edge-sentence')!.textContent).toBe('Jane and Mary are spouses');
    expect(row(el, ['relationships', 0, 'married'])).toBeTruthy();
    expect(edge(el, 0).querySelector('.fm-hint')).toBeNull();
  });

  it('adds an edge that writes {type, to: "[[X]]"}, leaving the others alone', async () => {
    const { el } = await mountMbr(text, (e) => {
      e.siteFiles = [{ url_path: '/people/sam/', frontmatter: { title: 'Sam' } }];
    });
    button(value(el, ['relationships']), 'Add relationship').click();
    await settle(el);
    expect(el.text).toBe(`${text}  - type:\n    to:\n`);
    commit(combo(edge(el, 1), '.fm-edge-type'), 'parent', 'pick');
    await settle(el);
    commit(combo(edge(el, 1), '.fm-wikilink'), '[[Sam]]', 'pick');
    await settle(el);
    expect(el.text).toBe(`${text}  - type: parent\n    to: "[[Sam]]"\n`);
    expect(edge(el, 1).querySelector('.fm-edge-sentence')!.textContent).toBe('Sam is Jane’s parent · Jane is Sam’s child');
  });

  it('reads [type] [note] with no to/from wording, and has no invert toggle for a symmetric type', async () => {
    const { el } = await mountMbr(text);
    expect(edge(el, 0).querySelector('select')).toBeNull();
    expect(edge(el, 0).querySelector('.fm-edge-dir-text')).toBeNull();
    expect(edge(el, 0).querySelector('.fm-edge-invert')).toBeNull();
    expect(edge(el, 0).querySelectorAll('.fm-wikilink')).toHaveLength(1);
  });

  it('flips direction with one fixed checkbox: checked writes from, unchecked to', async () => {
    const parent = 'title: Jane\nrelationships:\n  - type: parent\n    to: "[[Sam]]"\n';
    const { el } = await mountMbr(parent);
    const box = () => edge(el, 0).querySelector<HTMLInputElement>('.fm-edge-invert input[type="checkbox"]')!;
    expect(edge(el, 0).querySelector('.fm-edge-invert')!.textContent!.trim()).toBe('Flip relationship direction');
    expect(box().checked).toBe(false);
    expect(box().getAttribute('aria-label')).toBe('Relationship 1: flip relationship direction');
    box().checked = true;
    box().dispatchEvent(new Event('change', { bubbles: true }));
    await settle(el);
    expect(el.text).toBe(parent.replace('to: "[[Sam]]"', 'from: "[[Sam]]"'));
    expect(box().checked).toBe(true);
    expect(edge(el, 0).querySelector('.fm-edge-sentence')!.textContent).toBe('Jane is Sam’s parent · Sam is Jane’s child');
    box().checked = false;
    box().dispatchEvent(new Event('change', { bubbles: true }));
    await settle(el);
    expect(el.text).toBe(parent);
  });

  it('uses the same wording for every type, checked for a from edge', async () => {
    const { el } = await mountMbr('relationships:\n  - type: reports_to\n    from: "[[Al]]"\n');
    expect(edge(el, 0).querySelector('.fm-edge-invert')!.textContent!.trim()).toBe('Flip relationship direction');
    expect(edge(el, 0).querySelector<HTMLInputElement>('.fm-edge-invert input')!.checked).toBe(true);
  });

  it('shows both endpoints, labelled, and no toggle for an edge between two other notes', async () => {
    const { el } = await mountMbr('relationships:\n  - type: parent\n    from: "[[A]]"\n    to: "[[B]]"\n');
    expect([...edge(el, 0).querySelectorAll('.fm-edge-dir-text')].map((t) => t.textContent)).toEqual(['from', 'to']);
    expect([...edge(el, 0).querySelectorAll<MbrFmCombobox>('.fm-wikilink')].map((c) => c.value)).toEqual(['[[A]]', '[[B]]']);
    expect(edge(el, 0).querySelector('.fm-edge-invert')).toBeNull();
  });

  it('keeps a symmetric edge written with from as it is', async () => {
    const { el } = await mountMbr('title: Jane\nrelationships:\n  - type: sibling\n    from: "[[Al]]"\n');
    expect(edge(el, 0).querySelector('.fm-edge-invert')).toBeNull();
    expect(combo(edge(el, 0), '.fm-wikilink').value).toBe('[[Al]]');
    expect(edge(el, 0).querySelector('.fm-edge-sentence')!.textContent).toBe('Al and Jane are siblings');
  });

  it('adds a label and an attribute in place', async () => {
    const { el } = await mountMbr(text);
    type(edge(el, 0).querySelector<HTMLInputElement>('input.fm-edge-label')!, 'Wife');
    await settle(el);
    commit(combo(edge(el, 0), '.fm-edge-add mbr-fm-combobox'), 'place');
    await settle(el);
    expect(el.text).toBe(`${text}    label: Wife\n    place:\n`);
  });

  it('allows a type the registry does not know, with a soft hint', async () => {
    const { el } = await mountMbr('relationships:\n  - type: mentor\n    to: "[[Yoda]]"\n');
    expect(edge(el, 0).querySelector('.fm-hint')!.textContent).toContain('not in relationship_types');
    expect(edge(el, 0).querySelector('.fm-edge-sentence')!.textContent).toBe('Yoda is this note’s mentor');
  });

  it('removes an edge', async () => {
    const { el } = await mountMbr(`${text}  - type: child\n    to: "[[Al]]"\n`);
    edge(el, 0).querySelector<HTMLButtonElement>('[aria-label="Remove relationship 1"]')!.click();
    await settle(el);
    expect(el.text).toBe('title: Jane\nrelationships:\n  - type: child\n    to: "[[Al]]"\n');
  });
});

describe('labeled contact fields', () => {
  const labeledList = (el: MbrFmEditor, key: string) => value(el, [key]).querySelector('ol.fm-labeled')!;

  it('keeps a map a map', async () => {
    const { el } = await mountMbr('type: person\nphones:\n  mobile: "+1 555"\n');
    button(value(el, ['phones']), 'Add phone').click();
    await settle(el);
    expect(el.text).toBe('type: person\nphones:\n  mobile: "+1 555"\n  main:\n');
    type(labeledList(el, 'phones').querySelector<HTMLInputElement>('li[data-index="1"] .fm-labeled-value input')!, '+15550001');
    await settle(el);
    expect(el.text).toBe('type: person\nphones:\n  mobile: "+1 555"\n  main: "+15550001"\n');
  });

  it('keeps a list a list, relabels in place', async () => {
    const { el } = await mountMbr('type: person\nemails:\n  - work: a@x\n');
    button(value(el, ['emails']), 'Add email').click();
    await settle(el);
    expect(el.text).toBe('type: person\nemails:\n  - work: a@x\n  - home:\n');
    commit(combo(labeledList(el, 'emails'), 'li[data-index="0"] .fm-label'), 'school', 'pick');
    await settle(el);
    expect(el.text).toBe('type: person\nemails:\n  - school: a@x\n  - home:\n');
  });

  it('labels a bare alias, for any note type', async () => {
    const { el } = await mountMbr('aliases:\n  - Bob\n');
    commit(combo(labeledList(el, 'aliases'), '.fm-label'), 'nickname');
    await settle(el);
    expect(el.text).toBe('aliases:\n  - nickname: Bob\n');
  });

  it('edits a partial date under dates', async () => {
    const { el } = await mountMbr('type: person\ndates:\n  birthday: 1927-03-19\n');
    const year = labeledList(el, 'dates').querySelector<HTMLInputElement>('[data-part="year"]')!;
    expect(year.value).toBe('1927');
    const part = (name: string) => labeledList(el, 'dates').querySelector<HTMLSelectElement>(`[data-part="${name}"]`)!;
    // happy-dom does not select an <option> by its attribute once the <select> has a value; a browser does.
    type(part('month'), '03', 'change');
    type(part('day'), '19', 'change');
    await settle(el);
    expect(el.text).toBe('type: person\ndates:\n  birthday: 1927-03-19\n');
    type(labeledList(el, 'dates').querySelector<HTMLInputElement>('[data-part="year"]')!, '');
    await settle(el);
    expect(el.text).toBe('type: person\ndates:\n  birthday: --03-19\n');
  });

  it('edits an address part by part', async () => {
    const { el } = await mountMbr('type: person\naddresses:\n  home:\n    city: Paris\n');
    const list = labeledList(el, 'addresses');
    expect(list.querySelector<HTMLInputElement>('.fm-address-city')!.value).toBe('Paris');
    type(list.querySelector<HTMLInputElement>('.fm-address-postcode')!, '75010');
    await settle(el);
    expect(el.text).toBe('type: person\naddresses:\n  home:\n    city: Paris\n    postcode: "75010"\n');
  });
});

describe('OKF groups', () => {
  it('adds a source starting with its resource', async () => {
    const { el } = await mountMbr('sources:\n  - resource: https://a.test\n    title: A\n');
    expect(row(el, ['sources', 0]).querySelector('.fm-summary')!.textContent).toBe('https://a.test');
    const children = row(el, ['sources']).querySelector('.fm-children')!;
    button(children, 'Add item').click();
    await settle(el);
    expect(el.text).toBe('sources:\n  - resource: https://a.test\n    title: A\n  - resource:\n');
    type(value(el, ['sources', 1, 'resource']).querySelector<HTMLInputElement>('input')!, 'docs/b.md');
    await settle(el);
    expect(el.text).toBe('sources:\n  - resource: https://a.test\n    title: A\n  - resource: docs/b.md\n');
  });

  it('keeps a single verified map until a second review is added', async () => {
    const text = 'verified:\n  by: human:pat\n  at: 2026-01-02T03:04:05Z\n';
    const { el } = await mountMbr(text);
    expect(el.text).toBe(text);
    button(row(el, ['verified']).querySelector('.fm-children')!, 'Add item').click();
    await settle(el);
    const v = (toValue(el.model) as { verified: unknown[] }).verified;
    expect(v).toHaveLength(2);
    expect(el.text.startsWith('verified:\n  - by: human:pat\n    at: 2026-01-02T03:04:05Z\n  - by:\n    at: ')).toBe(true);
    expect(el.model.normalized).toBe(false);
  });

  it('keeps a timestamp’s offset, and adds one to a new timestamp', async () => {
    const { el } = await mountMbr('stale_after: 2026-06-30T14:00:00-06:00\ngenerated:\n  by: x/1\n  at:\n');
    type(value(el, ['stale_after']).querySelector<HTMLInputElement>('input[type="datetime-local"]')!, '2026-07-01T09:30');
    await settle(el);
    expect(el.text).toContain('stale_after: 2026-07-01T09:30:00-06:00\n');
    type(value(el, ['generated', 'at']).querySelector<HTMLInputElement>('input[type="datetime-local"]')!, '2026-07-01T09:30');
    await settle(el);
    expect((toValue(el.model) as { generated: { at: string } }).generated.at).toMatch(/^2026-07-01T09:30:00(Z|[+-]\d{2}:\d{2})$/);
  });

  it('fills in actors, with a "me" remembered for the page', async () => {
    const memory = createActorMemory();
    const { el } = await mountMbr('generated:\n  by:\n  at:\nverified:\n  - by:\n', (e) => {
      e.actorMemory = memory;
    });
    const by = value(el, ['generated', 'by']);
    button(by, 'Me').click();
    await settle(el);
    // No id yet: the field is primed with `human:` and nothing is written until it is typed.
    expect(combo(by).value).toBe('human:');
    expect(el.text).toBe('generated:\n  by:\n  at:\nverified:\n  - by:\n');
    expect(row(el, ['generated', 'by']).querySelector('.fm-local-hint')).not.toBeNull();
    commit(combo(value(el, ['generated', 'by'])), 'human:pat');
    await settle(el);
    expect(memory.id).toBe('pat');
    button(value(el, ['verified', 0, 'by']), 'Me').click();
    await settle(el);
    expect(el.text).toBe('generated:\n  by: human:pat\n  at:\nverified:\n  - by: human:pat\n');
    expect(combo(value(el, ['verified', 0, 'by'])).options[0]).toEqual({ value: 'human:pat', detail: 'You' });
  });
});
