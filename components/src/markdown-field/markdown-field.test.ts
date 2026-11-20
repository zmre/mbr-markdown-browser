/**
 * `<mbr-markdown-field>`: value in and out, `change`/`blur`, `readonly`,
 * the placeholder, the round trip of mbr's markdown, and the promise that it
 * reads nothing of the page it is embedded in.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { TextSelection } from '@milkdown/kit/prose/state';
import type { EditorView } from '@milkdown/kit/prose/view';
import { openDocument } from '../editor-frontmatter.js';
import { fixture, press, typeText } from '../editor-test-support.js';
import {
  CHANGE_DEBOUNCE_MS,
  MbrMarkdownField,
  defineMarkdownField,
  fieldEditorView,
  type MarkdownFieldChangeDetail,
} from './mbr-markdown-field.js';

defineMarkdownField();

type FieldProps = Partial<Pick<MbrMarkdownField, 'value' | 'readonly' | 'placeholder' | 'catalogUrl' | 'linkBase'>>;

/** A connected field with `props`, its editor mounted. */
async function mountField(props: FieldProps = {}, attrs: Record<string, string> = {}): Promise<MbrMarkdownField> {
  const field = document.createElement('mbr-markdown-field');
  for (const [name, value] of Object.entries(attrs)) field.setAttribute(name, value);
  Object.assign(field, props);
  document.body.appendChild(field);
  await field.ready;
  return field;
}

function viewOf(field: MbrMarkdownField): EditorView {
  const view = fieldEditorView(field);
  if (!view) throw new Error('field not mounted');
  return view;
}

/** Puts the caret at the end of the text node containing `text`. */
function caretAfter(view: EditorView, text: string): void {
  let found = -1;
  view.state.doc.descendants((node, pos) => {
    if (found >= 0) return false;
    if (node.isText && node.text?.includes(text)) found = pos + node.text.indexOf(text) + text.length;
    return true;
  });
  if (found < 0) throw new Error(`no text ${text}`);
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, found)));
}

/** Changes the document and changes it back: the field is dirty, its content is not. */
function touch(view: EditorView, text: string): void {
  caretAfter(view, text);
  const at = view.state.selection.from;
  view.dispatch(view.state.tr.insertText('x', at));
  view.dispatch(view.state.tr.delete(at, at + 1));
}

/** Every `change` and `blur` the field sends, in order. */
function recordEvents(field: MbrMarkdownField): { log: string[]; values: string[] } {
  const log: string[] = [];
  const values: string[] = [];
  field.addEventListener('change', (event) => {
    log.push('change');
    values.push((event as CustomEvent<MarkdownFieldChangeDetail>).detail.value);
  });
  field.addEventListener('blur', () => log.push('blur'));
  return { log, values };
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

afterEach(() => {
  document.body.innerHTML = '';
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('<mbr-markdown-field> value', () => {
  it('opens on the value it is given and returns it untouched', async () => {
    const value = '# Notes\n\nThanks for your business.\n';
    const field = await mountField({ value });
    expect(field.value).toBe(value);
    expect(field.querySelector('.ProseMirror')?.textContent).toContain('Thanks for your business.');
  });

  it('takes its first value from the attribute', async () => {
    const field = await mountField({}, { value: 'From the *attribute*.' });
    expect(field.value).toBe('From the *attribute*.');
    expect(field.querySelector('em')?.textContent).toBe('attribute');
  });

  it('keeps a value set before the element was defined', async () => {
    // A framework that sets `value` before the class is defined leaves an own
    // property shadowing the accessor; connecting must route it through.
    const host = document.createElement('mbr-markdown-field');
    Object.defineProperty(host, 'value', { value: 'Set early.', writable: true, configurable: true, enumerable: true });
    document.body.appendChild(host);
    await host.ready;
    expect(Object.prototype.hasOwnProperty.call(host, 'value')).toBe(false);
    expect(host.value).toBe('Set early.');
  });

  it('reads an edit back at once, before any change event', async () => {
    const field = await mountField({ value: 'Pay within 30 days.\n' });
    caretAfter(viewOf(field), 'days.');
    typeText(viewOf(field), ' Thanks!');
    expect(field.value).toBe('Pay within 30 days. Thanks!\n');
  });

  it('replaces the document when the host sets a different value, without a change event', async () => {
    const field = await mountField({ value: 'First.' });
    const { log } = recordEvents(field);
    field.value = 'Second, **bold**.';
    await field.ready;
    expect(field.querySelector('strong')?.textContent).toBe('bold');
    expect(field.value).toBe('Second, **bold**.');
    await sleep(CHANGE_DEBOUNCE_MS + 20);
    expect(log).toEqual([]);
  });

  it('keeps the editor (and the caret) when the host writes back the value it was sent', async () => {
    const field = await mountField({ value: 'Draft' });
    const view = viewOf(field);
    caretAfter(view, 'Draft');
    typeText(view, ' two');
    field.value = field.value; // a two-way binding echoing `change`
    await field.ready;
    expect(fieldEditorView(field)).toBe(view);
    expect(view.state.selection.from).toBe(view.state.doc.content.size - 1);
  });

  it('keeps CRLF line endings', async () => {
    const field = await mountField({ value: 'One.\r\n\r\nTwo.\r\n' });
    caretAfter(viewOf(field), 'Two.');
    typeText(viewOf(field), '!');
    expect(field.value).toBe('One.\r\n\r\nTwo.!\r\n');
  });

  it('keeps what was typed when it is moved in the DOM', async () => {
    const field = await mountField({ value: 'Before' });
    caretAfter(viewOf(field), 'Before');
    typeText(viewOf(field), ' after');
    const other = document.createElement('section');
    document.body.appendChild(other);
    other.appendChild(field);
    await field.ready;
    expect(field.value).toBe('Before after');
    expect(field.querySelector('.ProseMirror')?.textContent).toBe('Before after');
  });
});

describe('<mbr-markdown-field> events', () => {
  it('sends one change with the value after typing settles', async () => {
    const field = await mountField({ value: 'Total due' });
    const { log, values } = recordEvents(field);
    caretAfter(viewOf(field), 'due');
    typeText(viewOf(field), ' now');
    expect(log).toEqual([]);
    await sleep(CHANGE_DEBOUNCE_MS + 40);
    expect(log).toEqual(['change']);
    expect(values).toEqual(['Total due now']);
  });

  it('bubbles change as a CustomEvent', async () => {
    const field = await mountField({ value: 'x' });
    const seen: Event[] = [];
    document.body.addEventListener('change', (event) => seen.push(event));
    caretAfter(viewOf(field), 'x');
    typeText(viewOf(field), 'y');
    field.dispatchEvent(new FocusEvent('focusout', { relatedTarget: null }));
    expect(seen).toHaveLength(1);
    expect(seen[0]).toBeInstanceOf(CustomEvent);
    expect((seen[0] as CustomEvent<MarkdownFieldChangeDetail>).detail).toEqual({ value: 'xy' });
  });

  it('sends a pending change before blur when focus leaves', async () => {
    const field = await mountField({ value: 'Memo' });
    const { log, values } = recordEvents(field);
    caretAfter(viewOf(field), 'Memo');
    typeText(viewOf(field), ' line');
    viewOf(field).dom.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: document.body }));
    expect(log).toEqual(['change', 'blur']);
    expect(values).toEqual(['Memo line']);
    await sleep(CHANGE_DEBOUNCE_MS + 20);
    expect(log).toEqual(['change', 'blur']);
  });

  it('sends blur alone when nothing changed', async () => {
    const field = await mountField({ value: 'Memo' });
    const { log } = recordEvents(field);
    viewOf(field).dom.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: null }));
    expect(log).toEqual(['blur']);
  });

  it('does not blur when focus moves to its own menus and popovers', async () => {
    const field = await mountField({ value: 'Memo' });
    const { log } = recordEvents(field);
    const popover = document.createElement('input');
    field.querySelector('[data-mbr-popover-host]')!.appendChild(popover);
    viewOf(field).dom.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: popover }));
    expect(log).toEqual([]);
  });

  it('sends no change for an edit undone before it settled', async () => {
    const field = await mountField({ value: 'Same' });
    const { log } = recordEvents(field);
    touch(viewOf(field), 'Same');
    await sleep(CHANGE_DEBOUNCE_MS + 40);
    expect(log).toEqual([]);
  });

  it('keeps native change events of its inner inputs from reaching the host', async () => {
    const field = await mountField({ value: 'x' });
    const { log } = recordEvents(field);
    const input = document.createElement('input');
    field.querySelector('.milkdown')!.appendChild(input);
    input.dispatchEvent(new Event('change', { bubbles: true }));
    expect(log).toEqual([]);
  });
});

describe('<mbr-markdown-field> readonly and placeholder', () => {
  it('is editable by default and read-only with the attribute', async () => {
    const field = await mountField({ value: 'Locked' });
    expect(viewOf(field).editable).toBe(true);
    field.setAttribute('readonly', '');
    expect(field.readonly).toBe(true);
    expect(viewOf(field).editable).toBe(false);
    expect(field.getAttribute('aria-readonly')).toBe('true');
    field.readonly = false;
    expect(field.hasAttribute('readonly')).toBe(false);
    expect(viewOf(field).editable).toBe(true);
  });

  it('starts read-only when the property is set before connecting', async () => {
    const field = await mountField({ value: 'Locked', readonly: true });
    expect(field.hasAttribute('readonly')).toBe(true);
    expect(viewOf(field).editable).toBe(false);
  });

  it('shows the placeholder while empty, and a changed one keeps the value', async () => {
    const field = await mountField({ value: '', placeholder: 'Notes above the table' });
    const shown = () => field.querySelector('[data-placeholder]')?.getAttribute('data-placeholder');
    expect(shown()).toBe('Notes above the table');
    field.placeholder = 'Notes below';
    await field.ready;
    expect(shown()).toBe('Notes below');
    expect(field.getAttribute('placeholder')).toBe('Notes below');
  });
});

describe('<mbr-markdown-field> markdown fidelity', () => {
  // The constructs a host's prose is most likely to carry, from the editor's
  // parity corpus (tests/fixtures/editor-roundtrip/). An edit that changes
  // nothing still sends the whole document through the save path.
  const FIXTURES = [
    'wikilinks',
    'alerts',
    'math',
    'math-fences',
    'deflist-spelling',
    'deflist-edge',
    'footnotes',
    'tasks',
    'tables',
  ];

  for (const name of FIXTURES) {
    it(`round-trips ${name}.md byte for byte through the save path`, async () => {
      const { body } = openDocument(fixture(name));
      const field = await mountField({ value: body });
      const view = viewOf(field);
      let firstText = '';
      view.state.doc.descendants((node) => {
        if (!firstText && node.isText && node.text) firstText = node.text.slice(0, 1);
        return !firstText;
      });
      touch(view, firstText);
      expect(field.value).toBe(body);
    });
  }

  it('keeps wikilinks, alerts, math and definition lists verbatim around an edit', async () => {
    const value = [
      'See [[Acme Corp|the client]] and [[Projects/Q4#Scope]].',
      '',
      '> [!NOTE]',
      '> Late fees apply after 30 days.',
      '',
      'Rate: $r = 1.5\\%$ per month, so',
      '',
      '$$',
      '\\text{fee} = \\text{balance} \\times r',
      '$$',
      '',
      'Net 30',
      ': Payment due thirty days after the invoice date.',
      '',
      'Edit me.',
      '',
    ].join('\n');
    const field = await mountField({ value });
    caretAfter(viewOf(field), 'Edit me.');
    typeText(viewOf(field), ' Done');
    expect(field.value).toBe(value.replace('Edit me.', 'Edit me. Done'));
  });

  it('writes no HTML the author did not type', async () => {
    const field = await mountField({ value: 'Line' });
    const view = viewOf(field);
    caretAfter(view, 'Line');
    press(view, 'Enter');
    press(view, 'Enter');
    typeText(view, 'After gap');
    expect(field.value).not.toMatch(/<br|&#x20;|<\/?p>/);
    expect(field.value).toContain('After gap');
  });

  it('inserts a wikilink from the catalog', async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ markdown_files: [{ url_path: '/customers/acme/', frontmatter: { title: 'Acme Corp' } }] })),
    );
    vi.stubGlobal('fetch', fetchMock);
    const field = await mountField({ value: 'Bill to', catalogUrl: '/catalog.json' });
    const view = viewOf(field);
    caretAfter(view, 'Bill to');
    typeText(view, ' [[Acm');
    await sleep(0);
    await sleep(0);
    const popup = field.querySelector('.mbr-autocomplete');
    expect(popup, 'the popup attaches inside the themed field').not.toBeNull();
    expect(popup?.textContent).toContain('Acme Corp');
    view.dom.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(field.value).toBe('Bill to [[Acme Corp]]');
    expect(fetchMock).toHaveBeenCalledWith('/catalog.json', { credentials: 'same-origin' });
  });
});

describe('<mbr-markdown-field> reads nothing of the page', () => {
  it('touches no mbr page global, no storage and no network without a catalog', async () => {
    const touched: string[] = [];
    for (const name of ['frontmatter', '__MBR_CONFIG__', '__MBR_EXPORT__', 'ipc']) {
      Object.defineProperty(window, name, {
        configurable: true,
        get() {
          touched.push(name);
          return undefined;
        },
      });
    }
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    vi.stubGlobal('WebSocket', vi.fn());
    const getItem = vi.spyOn(Storage.prototype, 'getItem');
    const setItem = vi.spyOn(Storage.prototype, 'setItem');
    try {
      const field = await mountField({ value: 'Hello [[World]]' });
      const view = viewOf(field);
      caretAfter(view, 'Hello ');
      typeText(view, 'there [[Wo');
      view.dom.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: null }));
      void field.value;
      expect(touched).toEqual([]);
      expect(fetchMock).not.toHaveBeenCalled();
      expect(getItem).not.toHaveBeenCalled();
      expect(setItem).not.toHaveBeenCalled();
      expect(field.querySelector('.mbr-autocomplete')).toBeNull();
    } finally {
      for (const name of ['frontmatter', '__MBR_CONFIG__', '__MBR_EXPORT__', 'ipc']) {
        delete (window as unknown as Record<string, unknown>)[name];
      }
    }
  });

  // The runtime check above covers one session; this one covers every line
  // the bundle ships. Each module the entry reaches is read as source (as
  // edit-token.test.ts does) and must not name anything that belongs to an
  // mbr page, its server or its other bundles.
  const SOURCES = import.meta.glob('../**/*.ts', { query: '?raw', import: 'default', eager: true }) as Record<
    string,
    string
  >;

  /** Keyed by path relative to `src/`: `../a/b.ts` → `a/b.ts`, and vite's `./x.ts` (this directory) → `markdown-field/x.ts`. */
  const sources = new Map(
    Object.entries(SOURCES).map(([path, text]) => [
      path.startsWith('./') ? `markdown-field/${path.slice(2)}` : path.replace(/^\.\.\//, ''),
      text,
    ]),
  );

  function stripComments(source: string): string {
    return source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:'"`])\/\/.*$/gm, '$1');
  }

  function resolveImport(from: string, spec: string): string | null {
    if (!spec.startsWith('.')) return null; // a package
    const parts = from.split('/').slice(0, -1);
    for (const segment of spec.replace(/\?.*$/, '').split('/')) {
      if (segment === '..') parts.pop();
      else if (segment !== '.') parts.push(segment);
    }
    return parts.join('/').replace(/\.js$/, '.ts');
  }

  /** Every module the bundle entry reaches through relative imports. */
  function bundleModules(entry: string): string[] {
    const seen = new Set<string>();
    const queue = [entry];
    while (queue.length > 0) {
      const file = queue.pop()!;
      if (seen.has(file)) continue;
      const text = sources.get(file);
      if (text === undefined) throw new Error(`${file} is imported but not found`);
      seen.add(file);
      for (const match of text.matchAll(/(?:from|import)\s*\(?\s*['"]([^'"]+)['"]/g)) {
        const target = resolveImport(file, match[1]);
        if (target && target.endsWith('.ts')) queue.push(target);
      }
    }
    return [...seen].sort();
  }

  const modules = bundleModules('markdown-field/index.ts');

  it('walks the whole editor core, so the scan cannot pass vacuously', () => {
    expect(modules).toContain('markdown-field/mbr-markdown-field.ts');
    expect(modules).toContain('editor-core.ts');
    expect(modules).toContain('editor-plugins/wikilinks.ts');
    expect(modules).toContain('editor-plugins/definition-list.ts');
    expect(modules).toContain('editor-link-autocomplete.ts');
    expect(modules.length).toBeGreaterThan(25);
  });

  it('includes none of the page-bound modules', () => {
    for (const bound of ['editor-crepe.ts', 'shared.ts', 'edit-token.ts', 'mbr-live-reload.ts', 'task-toggle.ts']) {
      expect(modules).not.toContain(bound);
    }
  });

  it('names no page global, storage, socket or mbr endpoint', () => {
    const FORBIDDEN = [
      'window.frontmatter',
      '__MBR_CONFIG__',
      '__MBR_EXPORT__',
      'window.location',
      'localStorage',
      'sessionStorage',
      'WebSocket',
      '/.mbr/',
      'window.ipc',
    ];
    const hits = modules.flatMap((file) => {
      const code = stripComments(sources.get(file) ?? '');
      return FORBIDDEN.filter((needle) => code.includes(needle)).map((needle) => `${file}: ${needle}`);
    });
    expect(hits).toEqual([]);
  });
});
