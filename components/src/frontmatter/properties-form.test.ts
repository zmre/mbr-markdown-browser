import { afterEach, describe, expect, it } from 'vitest';
import './properties-form.js';
import { cmType, cmView, commit, menuItem, mount, openMenu, row, settle, type, value } from './test-support.js';
import { builtinCatalog } from './fields.js';
import { inferCatalog } from './infer.js';
import type { MbrFmCombobox } from './fm-combobox.js';
import { FM_STYLE_ID } from './fm-styles.js';

afterEach(() => {
  document.body.innerHTML = '';
});

describe('<mbr-fm-editor>', () => {
  it('injects its stylesheet once and renders light DOM', async () => {
    await mount('a: 1');
    await mount('b: 2');
    expect(document.querySelectorAll(`#${FM_STYLE_ID}`)).toHaveLength(1);
    expect(document.querySelector('mbr-fm-editor')!.shadowRoot).toBeNull();
  });

  it('chooses a widget per kind', async () => {
    const text = [
      'title: Hello',
      'description: |',
      '  Two',
      '  lines',
      'order: 3',
      'pinned: true',
      'date: 2026-10-21',
      'stale_after: 2026-06-30T14:00:00Z',
      'type: person',
      'born: 1927',
      'colors:',
      '  - red',
      '  - green',
      'tags: [a, b]',
      'meta:',
      '  a: 1',
    ].join('\n');
    const { el } = await mount(text);
    expect(value(el, ['title']).querySelector('input[type="text"]')).not.toBeNull();
    expect(value(el, ['description']).querySelector('textarea')).not.toBeNull();
    expect(value(el, ['order']).querySelector('input[type="number"]')).not.toBeNull();
    expect(value(el, ['pinned']).querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true);
    expect(value(el, ['date']).querySelector<HTMLInputElement>('input[type="date"]')!.value).toBe('2026-10-21');
    expect(value(el, ['stale_after']).querySelector<HTMLInputElement>('input[type="datetime-local"]')!.value).toBe('2026-06-30T14:00');
    expect(value(el, ['stale_after']).querySelector('.fm-offset')!.textContent).toBe('UTC');
    expect(value(el, ['type']).querySelector('mbr-fm-combobox')).not.toBeNull();
    expect(value(el, ['born']).querySelector<HTMLInputElement>('[data-part="year"]')!.value).toBe('1927');
    expect(value(el, ['colors']).querySelectorAll('ol.fm-list > li')).toHaveLength(2);
    expect([...value(el, ['tags']).querySelectorAll('.fm-chip > span')].map((s) => s.textContent)).toEqual(['a', 'b']);
    expect(value(el, ['meta']).querySelector('[aria-expanded="true"]')).not.toBeNull();
    expect(row(el, ['meta', 'a'])).toBeTruthy();
  });

  it('leaves untouched frontmatter byte for byte', async () => {
    const text = "title:   'Spaced'   # note\n\n# a comment\ntags: [a,b]\n";
    const { el, changes } = await mount(text);
    expect(el.text).toBe(text);
    expect(changes).toEqual([]);
  });

  it('edits a scalar in place, keeping its comment and the rest of the text', async () => {
    const { el, changes } = await mount("title: 'Old' # keep\nn: 1\n");
    type(value(el, ['title']).querySelector('input')!, 'New');
    expect(el.text).toBe("title: 'New' # keep\nn: 1\n");
    expect(changes).toEqual([el.text]);
    type(value(el, ['title']).querySelector('input')!, '');
    expect(el.text).toBe('title: # keep\nn: 1\n');
  });

  it('writes numbers and checkboxes as their types', async () => {
    const { el } = await mount('order: 3\npinned: false');
    type(value(el, ['order']).querySelector('input')!, '4.5');
    const box = value(el, ['pinned']).querySelector<HTMLInputElement>('input')!;
    box.checked = true;
    box.dispatchEvent(new Event('change', { bubbles: true }));
    expect(el.text).toBe('order: 4.5\npinned: true');
  });

  it('adds a property with a typed empty value and focuses it', async () => {
    const { el } = await mount('title: x');
    el.openAddProperty();
    await settle(el);
    const combo = el.querySelector<MbrFmCombobox>('.fm-add mbr-fm-combobox')!;
    expect(combo.options.map((o) => o.value)).toContain('pinned');
    expect(combo.options.map((o) => o.value)).not.toContain('title');
    commit(combo, 'pinned');
    await settle(el);
    expect(el.text).toBe('title: x\npinned: false');
    expect(document.activeElement).toBe(value(el, ['pinned']).querySelector('input'));
    el.openAddProperty();
    await settle(el);
    commit(el.querySelector<MbrFmCombobox>('.fm-add mbr-fm-combobox')!, 'tags', 'tab');
    await settle(el);
    expect(el.text).toBe('title: x\npinned: false\ntags:');
    // An empty tags property already shows the tags widget.
    expect(value(el, ['tags']).querySelector('.fm-chips')).not.toBeNull();
  });

  it('refuses a property that exists, without writing', async () => {
    const { el, changes } = await mount('title: x');
    el.openAddProperty();
    await settle(el);
    commit(el.querySelector<MbrFmCombobox>('.fm-add mbr-fm-combobox')!, 'title');
    await settle(el);
    expect(changes).toEqual([]);
    expect(el.querySelector('.fm-status')!.textContent).toMatch(/already a property/);
  });

  it('opens "+ Add property" on Cmd/Ctrl+;', async () => {
    const { el } = await mount('a: 1');
    el.querySelector('input')!.dispatchEvent(new KeyboardEvent('keydown', { key: ';', ctrlKey: true, bubbles: true }));
    await settle(el);
    expect(el.querySelector('.fm-add mbr-fm-combobox')).not.toBeNull();
  });

  it('removes and renames through the row menu', async () => {
    const { el } = await mount('a: 1 # one\nb: 2\nc: 3\n');
    menuItem(await openMenu(el, ['b']), 'Remove').click();
    await settle(el);
    expect(el.text).toBe('a: 1 # one\nc: 3\n');
    menuItem(await openMenu(el, ['a']), 'Rename').click();
    await settle(el);
    const input = row(el, ['a']).querySelector<HTMLInputElement>('.fm-rename')!;
    expect(document.activeElement).toBe(input);
    input.value = 'alpha';
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await settle(el);
    expect(el.text).toBe('alpha: 1 # one\nc: 3\n');
  });

  it('closes the row menu on Escape without letting the editor see it', async () => {
    const { el } = await mount('a: 1');
    await openMenu(el, ['a']);
    let reached = false;
    const onDoc = () => {
      reached = true;
    };
    document.addEventListener('keydown', onDoc);
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    document.removeEventListener('keydown', onDoc);
    await settle(el);
    expect(reached).toBe(false);
    expect(el.querySelector('.fm-menu')).toBeNull();
  });

  it('offers only lossless type changes and applies them', async () => {
    const { el } = await mount('n: "42"\ns: hello');
    const sMenu = await openMenu(el, ['s']);
    expect(() => menuItem(sMenu, 'Number')).toThrow();
    menuItem(sMenu, 'Long text').click();
    await settle(el);
    expect(value(el, ['s']).querySelector('textarea')).not.toBeNull();
    expect(el.text).toBe('n: "42"\ns: hello');
    menuItem(await openMenu(el, ['n']), 'Number').click();
    await settle(el);
    expect(el.text).toBe('n: 42\ns: hello');
    expect(value(el, ['n']).querySelector('input[type="number"]')).not.toBeNull();
  });

  it('adds keys and items to nested values', async () => {
    const { el } = await mount('meta:\n  a: 1\nlist:\n  - x: 1\n');
    const addKeyBtn = [...row(el, ['meta']).querySelectorAll<HTMLButtonElement>('.fm-children .fm-link')].find((b) =>
      b.textContent?.includes('Add key'),
    )!;
    addKeyBtn.click();
    await settle(el);
    const combo = row(el, ['meta']).querySelector<MbrFmCombobox>('.fm-children mbr-fm-combobox')!;
    commit(combo, 'b');
    await settle(el);
    expect(el.text).toBe('meta:\n  a: 1\n  b:\nlist:\n  - x: 1\n');
    type(value(el, ['meta', 'b']).querySelector('input')!, 'two');
    expect(el.text).toBe('meta:\n  a: 1\n  b: two\nlist:\n  - x: 1\n');
    const addItemBtn = [...row(el, ['list']).querySelectorAll<HTMLButtonElement>('.fm-children .fm-link')].find((b) =>
      b.textContent?.includes('Add item'),
    )!;
    addItemBtn.click();
    await settle(el);
    expect(el.text).toBe('meta:\n  a: 1\n  b: two\nlist:\n  - x: 1\n  -\n');
  });

  it('edits a list of scalars, Enter in the last item adding the next', async () => {
    const { el } = await mount('colors:\n  - red\n  - green');
    const inputs = () => value(el, ['colors']).querySelectorAll<HTMLInputElement>('ol.fm-list input');
    type(inputs()[1], 'blue');
    expect(el.text).toBe('colors:\n  - red\n  - blue');
    inputs()[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await settle(el);
    expect(el.text).toBe('colors:\n  - red\n  - blue\n  -');
    expect(document.activeElement).toBe(inputs()[2]);
    type(inputs()[2], 'cyan');
    expect(el.text).toBe('colors:\n  - red\n  - blue\n  - cyan');
    value(el, ['colors']).querySelector<HTMLButtonElement>('li[data-index="0"] button')!.click();
    await settle(el);
    expect(el.text).toBe('colors:\n  - blue\n  - cyan');
  });

  it('adds tags from the field and suggests the vault’s', async () => {
    const { el } = await mount('tags: [a]', (e) => {
      e.inferred = inferCatalog([{ frontmatter: { tags: ['a', 'b', 'c'] } }, { frontmatter: { tags: ['b'] } }]);
    });
    const combo = value(el, ['tags']).querySelector<MbrFmCombobox>('mbr-fm-combobox')!;
    expect(combo.options.map((o) => o.value)).toEqual(['b', 'c']);
    commit(combo, 'b');
    await settle(el);
    expect(el.text).toBe('tags: [a, b]');
    commit(value(el, ['tags']).querySelector<MbrFmCombobox>('mbr-fm-combobox')!, 'b');
    expect(el.text).toBe('tags: [a, b]');
    value(el, ['tags']).querySelector<HTMLButtonElement>('.fm-chip button')!.click();
    await settle(el);
    expect(el.text).toBe('tags: [b]');
  });

  it('round-trips partial dates and refuses impossible ones', async () => {
    const { el } = await mount('type: person\nborn: 1927\n');
    const born = () => value(el, ['born']);
    const part = (name: string) => born().querySelector<HTMLInputElement | HTMLSelectElement>(`[data-part="${name}"]`)!;
    type(part('month'), '03', 'change');
    await settle(el);
    expect(el.text).toBe('type: person\nborn: 1927-03\n');
    type(part('day'), '19', 'change');
    await settle(el);
    expect(el.text).toBe('type: person\nborn: 1927-03-19\n');
    type(part('year'), '', 'input');
    await settle(el);
    expect(el.text).toBe('type: person\nborn: --03-19\n');
    type(part('month'), '', 'change');
    await settle(el);
    expect(el.text).toBe('type: person\nborn: --03-19\n');
    expect(row(el, ['born']).querySelector('.fm-local-hint')!.textContent).toBe('A day needs a month.');
  });

  it('keeps a timestamp’s offset when its time is edited', async () => {
    const { el } = await mount('generated:\n  by: human:pat\n  at: 2026-06-30T14:00:00+02:00\n');
    const input = value(el, ['generated', 'at']).querySelector<HTMLInputElement>('input[type="datetime-local"]')!;
    expect(value(el, ['generated', 'at']).querySelector('.fm-offset')!.textContent).toBe('UTC+02:00');
    type(input, '2026-07-01T09:30:00');
    expect(el.text).toBe('generated:\n  by: human:pat\n  at: 2026-07-01T09:30:00+02:00\n');
  });

  it('shows soft validation hints without blocking edits', async () => {
    const { el } = await mount('status: final\nstale_after: 2026-06-30T14:00:00');
    expect(row(el, ['status']).querySelector('.fm-hint')!.textContent).toMatch(/draft, stable or deprecated/);
    expect(row(el, ['stale_after']).querySelector('.fm-hint')!.textContent).toMatch(/UTC offset/);
  });

  it('is read-only over YAML that does not parse, and the YAML view fixes it', async () => {
    const { el, changes } = await mount('a: [1\nb: 2');
    expect(el.querySelectorAll('.fm-row')).toHaveLength(0);
    expect(el.querySelector('.fm-errors')!.textContent).toMatch(/Line 2, column 1/);
    expect(el.querySelector('.fm-add')).toBeNull();
    el.openAddProperty();
    await settle(el);
    expect(el.querySelector('.fm-add')).toBeNull();
    const fix = [...el.querySelectorAll<HTMLButtonElement>('.fm-link')].find((b) => b.textContent?.includes('Fix it in YAML'))!;
    fix.click();
    await settle(el);
    const view = cmView(el.querySelector('.fm-yaml-text'));
    expect(view.state.doc.toString()).toBe('a: [1\nb: 2');
    expect(view.contentDOM.getAttribute('aria-label')).toBe('Frontmatter YAML');
    cmType(view, 'a: [1]\nb: 2');
    expect(changes).toEqual(['a: [1]\nb: 2']);
    expect(el.text).toBe('a: [1]\nb: 2');
    el.querySelector<HTMLButtonElement>('[role="tab"]')!.click();
    await settle(el);
    expect(el.querySelectorAll('.fm-row')).toHaveLength(2);
    expect(el.querySelector('.cm-editor')).toBeNull();
  });

  it('keeps the YAML editor in step with a model replaced from outside, without reporting it as an edit', async () => {
    const { el, changes } = await mount('a: 1\n');
    el.tab = 'yaml';
    await settle(el);
    const view = cmView(el.querySelector('.fm-yaml-text'));
    el.text = 'b: 2\n';
    await settle(el);
    expect(view.state.doc.toString()).toBe('b: 2\n');
    expect(changes).toEqual([]);
    // Typed bytes are what a save writes, even ones the form would format differently.
    cmType(view, 'b:   2   # spaced\n');
    expect(el.text).toBe('b:   2   # spaced\n');
  });

  it('edits a value with anchors and comments as YAML, applied in place', async () => {
    const { el } = await mount('base: &b\n  x: 1\nother:\n  # why\n  y: 2\nz: 3\n');
    expect(value(el, ['base']).querySelector('.fm-preview')!.textContent).toBe('&b\nx: 1');
    expect(value(el, ['other']).querySelector('.fm-preview')!.textContent).toBe('# why\ny: 2');
    expect(row(el, ['other']).querySelector('.fm-children')).toBeNull();
    const edit = [...value(el, ['other']).querySelectorAll<HTMLButtonElement>('.fm-link')].find((b) => b.textContent?.includes('Edit as YAML'))!;
    edit.click();
    await settle(el);
    const slice = cmView(value(el, ['other']).querySelector('.fm-yaml-slice'));
    expect(slice.state.doc.toString()).toBe('# why\ny: 2');
    cmType(slice, '# why\ny: 3\nw: [4');
    await settle(el);
    [...value(el, ['other']).querySelectorAll<HTMLButtonElement>('.fm-link')].find((b) => b.textContent?.trim() === 'Apply')!.click();
    await settle(el);
    expect(value(el, ['other']).querySelector('[role="alert"]')).not.toBeNull();
    expect(el.text).toBe('base: &b\n  x: 1\nother:\n  # why\n  y: 2\nz: 3\n');
    expect(value(el, ['other']).querySelector('.fm-yaml-slice')!.getAttribute('data-invalid')).toBe('true');
    cmType(cmView(value(el, ['other']).querySelector('.fm-yaml-slice')), '# why\ny: 3');
    await settle(el);
    [...value(el, ['other']).querySelectorAll<HTMLButtonElement>('.fm-link')].find((b) => b.textContent?.trim() === 'Apply')!.click();
    await settle(el);
    expect(el.text).toBe('base: &b\n  x: 1\nother:\n  # why\n  y: 3\nz: 3\n');
  });

  it('uses the catalog it is given (tag sources)', async () => {
    const { el } = await mount('performers: [Ann]', (e) => {
      e.catalog = builtinCatalog({ tagSources: [{ field: 'performers', label: 'Performer', label_plural: 'Performers' }] });
    });
    expect(value(el, ['performers']).querySelector('.fm-chips')).not.toBeNull();
  });

  it('types a plain value with spaces and leaves it plain (text, long text, enum)', async () => {
    const { el } = await mount('title: x\nsummary: y\nstatus: draft\n');
    const typeEach = async (input: HTMLInputElement | HTMLTextAreaElement, text: string) => {
      for (let i = 1; i <= text.length; i++) {
        type(input, text.slice(0, i));
        await settle(el);
      }
    };
    await typeEach(value(el, ['title']).querySelector('input')!, 'hello world');
    expect(el.text).toBe('title: hello world\nsummary: y\nstatus: draft\n');
    await typeEach(value(el, ['summary']).querySelector('textarea')!, 'two words');
    expect(el.text).toBe('title: hello world\nsummary: two words\nstatus: draft\n');
    const combo = value(el, ['status']).querySelector<MbrFmCombobox>('mbr-fm-combobox')!;
    for (const prefix of ['s', 'st', 'st ', 'st a']) {
      combo.dispatchEvent(new CustomEvent('fm-combo-input', { detail: { value: prefix }, bubbles: true }));
      await settle(el);
    }
    expect(el.text).toBe('title: hello world\nsummary: two words\nstatus: st a\n');
  });

  it('lets a number be typed a keystroke at a time (1.05, -2), never writing a half-typed one', async () => {
    const { el, changes } = await mount('n: 3\n');
    const input = value(el, ['n']).querySelector<HTMLInputElement>('input[type="number"]')!;
    input.focus();
    // As a browser reports it: what is not a number yet is an empty value with `badInput`.
    const key = async (raw: string) => {
      const bad = raw !== '' && !/^-?\d+(\.\d+)?$/.test(raw);
      Object.defineProperty(input, 'validity', { value: { badInput: bad }, configurable: true });
      type(input, bad ? '' : raw);
      await settle(el);
    };
    await key('1');
    await key('1.');
    expect(el.text).toBe('n: 1\n');
    await key('1.0');
    expect(input.value).toBe('1.0');
    await key('1.05');
    expect(el.text).toBe('n: 1.05\n');
    expect(input.value).toBe('1.05');
    await key('-');
    expect(el.text).toBe('n: 1.05\n');
    await key('-2');
    expect(el.text).toBe('n: -2\n');
    expect(changes.some((t) => t === 'n:\n')).toBe(false);
    // Clearing the field is a clear.
    await key('');
    expect(el.text).toBe('n:\n');
    // A value changed elsewhere still shows.
    input.blur();
    el.text = 'n: 7\n';
    await settle(el);
    expect(value(el, ['n']).querySelector<HTMLInputElement>('input')!.value).toBe('7');
  });

  it('reports edits not yet applied, and flushes them', async () => {
    const { el } = await mount('tags: [a]\nname: x\nother: &o 1\n');
    expect(el.hasPendingEdits).toBe(false);
    // An open "Edit as YAML" with a change.
    const edit = [...value(el, ['other']).querySelectorAll<HTMLButtonElement>('.fm-link')].find((b) => b.textContent?.includes('Edit as YAML'))!;
    edit.click();
    await settle(el);
    cmType(cmView(value(el, ['other']).querySelector('.fm-yaml-slice')), '&o 2');
    expect(el.hasPendingEdits).toBe(true);
    // A tag typed without Enter.
    type(value(el, ['tags']).querySelector<MbrFmCombobox>('mbr-fm-combobox')!.querySelector('input')!, 'b');
    // A rename in progress (focus still in its field: nothing has blurred it).
    menuItem(await openMenu(el, ['name']), 'Rename').click();
    await settle(el);
    row(el, ['name']).querySelector<HTMLInputElement>('.fm-rename')!.value = 'title';
    expect(el.text).toBe('tags: [a]\nname: x\nother: &o 1\n');
    expect(el.hasPendingEdits).toBe(true);
    expect(el.flushPending()).toBeNull();
    await settle(el);
    expect(el.text).toBe('tags: [a, b]\ntitle: x\nother: &o 2\n');
    expect(el.hasPendingEdits).toBe(false);
  });

  it('keeps invalid YAML in an open slice pending and says why', async () => {
    const { el } = await mount('other: &o 1\n');
    [...value(el, ['other']).querySelectorAll<HTMLButtonElement>('.fm-link')].find((b) => b.textContent?.includes('Edit as YAML'))!.click();
    await settle(el);
    // Opened and left as it was: nothing pending.
    expect(el.hasPendingEdits).toBe(false);
    cmType(cmView(value(el, ['other']).querySelector('.fm-yaml-slice')), '[1');
    expect(el.flushPending()).toMatch(/other/);
    await settle(el);
    expect(el.text).toBe('other: &o 1\n');
    expect(el.hasPendingEdits).toBe(true);
    expect(value(el, ['other']).querySelector('[role="alert"]')).not.toBeNull();
  });

  it('moves collapsed rows and chosen kinds up with the items when a list item is removed', async () => {
    const own = (el: Element, path: (string | number)[]) => {
      const r = row(el as never, path);
      return [...r.querySelectorAll<HTMLButtonElement>('.fm-disclosure')].find((b) => b.closest('.fm-row') === r)!;
    };
    // Through the row menu: item 3 collapsed, item 1 removed.
    const { el } = await mount('l:\n  - {a: 1}\n  - {a: 2}\n  - {a: 3}\n  - {a: 4}\n');
    own(el, ['l', 2]).click();
    await settle(el);
    expect(own(el, ['l', 2]).getAttribute('aria-expanded')).toBe('false');
    menuItem(await openMenu(el, ['l', 0]), 'Remove').click();
    await settle(el);
    expect(el.text).toBe('l:\n  - {a: 2}\n  - {a: 3}\n  - {a: 4}\n');
    // Item 3 is now item 2, still collapsed; the new item 3 (was 4) did not inherit it.
    expect(own(el, ['l', 1]).getAttribute('aria-expanded')).toBe('false');
    expect(own(el, ['l', 2]).getAttribute('aria-expanded')).toBe('true');
    expect(own(el, ['l', 0]).getAttribute('aria-expanded')).toBe('true');

    // Through the list widget's own Remove: state below a later item moves up too.
    document.body.innerHTML = '';
    const { el: s } = await mount('n:\n  - a\n  - b\n  - c\n');
    s.setLocalHint(['n', 2], 'third');
    s.setLocalHint(['n', 1], 'second');
    value(s, ['n']).querySelector<HTMLButtonElement>('li[data-index="0"] button')!.click();
    await settle(s);
    expect(s.text).toBe('n:\n  - b\n  - c\n');
    const hints = (s as unknown as { _localHints: ReadonlyMap<string, string> })._localHints;
    expect([...hints]).toEqual([
      [JSON.stringify(['n', 1]), 'third'],
      [JSON.stringify(['n', 0]), 'second'],
    ]);
  });

  it('is read-only, not broken, over aliases that expand past the limit, and the YAML view still works', async () => {
    const laughs = ['a: &a [x, x, x, x, x, x, x, x, x]', 'b: &b [*a, *a, *a, *a, *a, *a, *a, *a, *a]', 'c: &c [*b, *b, *b, *b, *b, *b, *b, *b, *b]', 'd: &d [*c, *c, *c, *c, *c, *c, *c, *c, *c]', 'e: &e [*d, *d, *d, *d, *d, *d, *d, *d, *d]'].join('\n');
    const { el } = await mount(laughs);
    expect(el.querySelector('.fm-errors')!.textContent).toMatch(/alias/i);
    expect(el.querySelectorAll('.fm-row')).toHaveLength(0);
    expect(el.frontmatter).toEqual({});
    el.tab = 'yaml';
    await settle(el);
    const view = cmView(el.querySelector('.fm-yaml-text'));
    cmType(view, 'a: 1\n');
    el.tab = 'form';
    await settle(el);
    expect(el.querySelectorAll('.fm-row')).toHaveLength(1);
  });

  it('refuses to add a dotted property written as one flat key', async () => {
    const { el, changes } = await mount('type: person\ndates.birthday: 1927-03-19\n');
    el.openAddProperty();
    await settle(el);
    commit(el.querySelector<MbrFmCombobox>('.fm-add mbr-fm-combobox')!, 'dates.birthday');
    await settle(el);
    expect(changes).toEqual([]);
    expect(el.querySelector('.fm-status')!.textContent).toMatch(/already a property/);
  });

  it('builds note-link suggestions once per site index, not per field or render', async () => {
    const files = [{ url_path: '/a/', frontmatter: { title: 'A' } }, { url_path: '/b/', frontmatter: { title: 'B' } }];
    const { el } = await mount('company: "[[A]]"\ntype: person\n', (e) => {
      e.siteFiles = files;
    });
    const def = { key: 'company', kind: 'wikilink' as const, label: 'Company', group: 'contact' as const };
    const first = el.suggestions(['company'], def, 'wikilink');
    el.requestUpdate();
    await settle(el);
    const again = el.suggestions(['other'], def, 'wikilink');
    const link = (opts: typeof first) => opts.find((o) => o.value === '[[B]]');
    expect(link(first)).toBeDefined();
    expect(link(again)).toBe(link(first));
    el.siteFiles = [...files];
    expect(link(el.suggestions(['company'], def, 'wikilink'))).not.toBe(link(first));
  });

  it('offers an add affordance on an empty block', async () => {
    const { el } = await mount('');
    expect(el.querySelector('.fm-empty')!.textContent).toBe('No properties yet.');
    el.openAddProperty();
    await settle(el);
    commit(el.querySelector<MbrFmCombobox>('.fm-add mbr-fm-combobox')!, 'title');
    await settle(el);
    type(value(el, ['title']).querySelector('input')!, 'Hello');
    expect(el.text).toBe('title: Hello');
  });
});
