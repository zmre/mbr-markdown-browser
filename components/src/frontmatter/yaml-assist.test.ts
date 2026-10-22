import { afterEach, describe, expect, it } from 'vitest';
import { completionStatus, startCompletion, type Completion } from '@codemirror/autocomplete';
import { yaml } from '@codemirror/lang-yaml';
import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { builtinCatalog, type Frontmatter } from './fields.js';
import { inferCatalog } from './infer.js';
import { createActorMemory } from './mbr-logic.js';
import { yamlCompletions, yamlDiagnostics, yamlScalarText, type YamlAssistOptions, type YamlKnowledge } from './yaml-assist.js';
import { createYamlEditor, type YamlSourceEditor } from './yaml-source.js';

const SITE = [
  { url_path: '/people/alice/', frontmatter: { title: 'Alice Smith', type: 'person', tags: ['family', 'friends'] } },
  { url_path: '/orgs/acme/', frontmatter: { title: 'Acme Corp', type: 'organization', tags: 'work, clients' } },
  { url_path: '/notes/q3/', frontmatter: { title: 'Q3 plan', type: 'Playbook', status: 'draft', generated: { by: 'process:etl' } } },
  { url_path: '/notes/q4/', frontmatter: { title: 'Say "hi"', type: 'Playbook', tags: ['work'], 'taxonomy.genre': 'jazz' } },
];

const knowledge = (over: Partial<YamlKnowledge> = {}): YamlKnowledge => ({
  catalog: builtinCatalog({
    relationshipTypes: [
      { name: 'parent', inverse: 'child', symmetric: false, label: 'Parent', label_plural: 'Parents', category: 'family' },
      { name: 'mentor', inverse: 'mentee', symmetric: false, label: 'Mentor', label_plural: 'Mentors' },
    ],
  }),
  inferred: inferCatalog(SITE),
  siteFiles: SITE,
  actorMemory: createActorMemory(),
  ...over,
});

/** The completions at the last `|` in `src`, explicitly requested unless `typed`. */
function complete(src: string, opts: Partial<YamlAssistOptions> = {}, typed = false) {
  const pos = src.lastIndexOf('|');
  const doc = src.slice(0, pos) + src.slice(pos + 1);
  const state = EditorState.create({ doc, extensions: [yaml()] });
  const k = knowledge();
  return yamlCompletions(state, pos, !typed, { knowledge: () => k, ...opts });
}

/** The text after applying the completion labelled `label` (or shown as it) at the last `|`. */
function applied(src: string, label: string, opts: Partial<YamlAssistOptions> = {}, k: YamlKnowledge = knowledge()): string {
  const pos = src.lastIndexOf('|');
  const doc = src.slice(0, pos) + src.slice(pos + 1);
  const state = EditorState.create({ doc, extensions: [yaml()] });
  const res = yamlCompletions(state, pos, true, { knowledge: () => k, ...opts })!;
  const opt = res.options.find((o) => o.label === label || o.displayLabel === label);
  if (!opt) throw new Error(`no ${label} at ${src}`);
  const view = new EditorView({ state });
  const to = res.to ?? pos;
  if (typeof opt.apply === 'string') view.dispatch({ changes: { from: res.from, to, insert: opt.apply } });
  else (opt.apply as (v: EditorView, c: Completion, f: number, t: number) => void)(view, opt, res.from, to);
  const out = view.state.doc.toString();
  view.destroy();
  return out;
}

const labels = (src: string, opts?: Partial<YamlAssistOptions>) => complete(src, opts)?.options.map((o) => o.label) ?? [];
const option = (src: string, label: string, opts?: Partial<YamlAssistOptions>): Completion => {
  const found = complete(src, opts)?.options.find((o) => o.label === label);
  if (!found) throw new Error(`no ${label} at ${src}`);
  return found;
};

describe('key completions', () => {
  it('offers built-ins first, then the vault’s keys, without those present', () => {
    const keys = labels('title: x\n|');
    expect(keys.slice(0, 3)).toEqual(['description', 'summary', 'type']);
    expect(keys).toContain('status');
    expect(keys).not.toContain('title');
    expect(keys).not.toContain('first_name'); // contact fields only for type: person
    expect(labels('type: person\n|')).toContain('first_name');
    expect(labels('type: Attested Computation\n|')).toContain('runtime');
  });

  it('offers a dotted vault key as its first key', () => {
    expect(labels('|')).toContain('taxonomy');
  });

  it('puts help in the info and the label in the detail', () => {
    const type = option('|', 'type');
    expect(type.detail).toBe('Type');
    expect(type.info).toMatch(/kind of note/);
    expect(option('|', 'status').info).toMatch(/OKF/);
  });

  it('offers a nested map’s known keys, and labels for labeled fields', () => {
    expect(labels('generated:\n  |')).toEqual(['by', 'at']);
    expect(labels('generated:\n  by: human:me\n  |')).toEqual(['at']);
    expect(labels('relationships:\n  - type: parent\n    |')).toEqual(['to', 'from', 'label']);
    expect(labels('sources:\n  - resource: x\n    |')).toEqual(expect.arrayContaining(['id', 'title', 'author', 'usage_count']));
    expect(labels('type: person\nphones:\n  |')).toEqual(['mobile', 'main', 'home', 'work', 'home_fax', 'work_fax', 'pager', 'other']);
    expect(labels('type: person\nphones:\n  mobile: "+1"\n  |')).not.toContain('mobile');
    expect(labels('type: person\ndates:\n  |')).toEqual(['birthday', 'death', 'anniversary']);
    expect(labels('verified:\n  by: x\n  |')).toEqual(['at']);
  });

  it('offers the vault’s nested keys under their parent', () => {
    expect(labels('taxonomy:\n  |')).toEqual(['genre']);
  });

  it('inserts the start of the value in the shape the field takes', () => {
    const insertOf = (src: string, label: string) => {
      const c = complete(src)!;
      const opt = c.options.find((o) => o.label === label)!;
      const doc = src.replace('|', '');
      const view = new EditorView({ state: EditorState.create({ doc, extensions: [yaml()] }) });
      (opt.apply as (v: EditorView, c: Completion, f: number, t: number) => void)(view, opt, c.from, c.to ?? c.from);
      const out = view.state.doc.toString();
      view.destroy();
      return out;
    };
    expect(insertOf('ty|', 'type')).toBe('type: ');
    expect(insertOf('gen|', 'generated')).toBe('generated:\n  ');
    expect(insertOf('rel|', 'relationships')).toBe('relationships:\n  - ');
    expect(insertOf('m:\n  x: 1\nsou|', 'sources')).toBe('m:\n  x: 1\nsources:\n  - ');
    expect(insertOf('sta|tus: draft', 'status')).toBe('status: draft');
  });

  it('a slice’s own keys are relative to its value', () => {
    expect(labels('by: x\n|', { basePath: ['generated'], frontmatter: () => ({ generated: { by: 'x' } }) })).toEqual(['at']);
  });
});

describe('value completions', () => {
  it('type: mbr’s types with help, then the vault’s', () => {
    const c = complete('type: |')!;
    expect(c.options.map((o) => o.label)).toEqual(['slides', 'flashcard', 'person', 'organization', 'Playbook']);
    expect(c.options[0].info).toMatch(/slideshow/);
    expect(c.options[0].detail).toBeUndefined();
    expect(c.options[4].detail).toBe('2 notes');
  });

  it('status: the OKF values', () => {
    expect(labels('status: |')).toEqual(['draft', 'stable', 'deprecated']);
    expect(option('status: |', 'stable').info).toMatch(/default when absent/);
  });

  it('tags: the vault’s tags, in a list, a flow list and a comma string, without those already there', () => {
    // Most used first, then by name.
    expect(labels('tags: [|]')).toEqual(['work', 'clients', 'family', 'friends']);
    expect(labels('tags: [work, |]')).not.toContain('work');
    expect(labels('tags:\n  - family\n  - |')).not.toContain('family');
    const comma = complete('tags: work, cl|')!;
    expect(comma.from).toBe('tags: work, '.length);
    expect(comma.options.map((o) => o.label)).toContain('clients');
  });

  it('actors: the vault’s, the reader’s own and the OKF prefixes (left unquoted to type on)', () => {
    const memory = createActorMemory();
    memory.id = 'pat';
    const k = knowledge({ actorMemory: memory });
    const state = EditorState.create({ doc: 'generated:\n  by: ', extensions: [yaml()] });
    const c = yamlCompletions(state, state.doc.length, true, { knowledge: () => k })!;
    expect(c.options.map((o) => o.label)).toEqual(['human:pat', 'process:etl', 'human:', 'process:']);
    expect(applied('generated:\n  by: |', 'human:', {}, k)).toBe('generated:\n  by: human:');
    expect(labels('verified:\n  - by: |')).toContain('process:etl');
  });

  it('relationship types from the registry, or mbr’s defaults without one', () => {
    expect(labels('relationships:\n  - type: |')).toEqual(['parent', 'mentor']);
    expect(option('relationships:\n  - type: |', 'parent').detail).toBe('Parent · family');
    const none = { knowledge: () => knowledge({ catalog: builtinCatalog() }) };
    expect(labels('relationships:\n  - type: |', none)).toContain('colleague');
    expect(labels('relationships: [{type: |}]')).toEqual(['parent', 'mentor']);
  });

  it('note links for a link field are quoted, since `[[x]]` unquoted is a nested list', () => {
    const c = complete('type: person\ncompany: |')!;
    const acme = c.options.find((o) => o.label === '[[Acme Corp]]')!;
    expect(acme.displayLabel).toBe('Acme Corp');
    expect(applied('type: person\ncompany: |', '[[Acme Corp]]')).toBe('type: person\ncompany: "[[Acme Corp]]"');
    expect(applied('type: person\ncompany: "|"', '[[Acme Corp]]')).toBe('type: person\ncompany: "[[Acme Corp]]"');
  });

  it('booleans, dates and datetimes', () => {
    expect(labels('pinned: |')).toEqual(['true', 'false']);
    expect(labels('date: |')[0]).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(labels('stale_after: |')[0]).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(Z|[+-]\d{2}:\d{2})$/);
  });

  it('labels of `{label, value}` entries', () => {
    expect(labels('type: person\nemails:\n  - label: |')).toEqual(['home', 'work', 'other', 'school']);
  });

  it('a value edited on its own (Edit as YAML) completes as the value it is', () => {
    const opts = { basePath: ['status'], frontmatter: (): Frontmatter => ({ status: 'draft' }) };
    expect(labels('dr|', opts)).toEqual(['draft', 'stable', 'deprecated']);
    const tags = { basePath: ['tags'], frontmatter: (): Frontmatter => ({ tags: ['work'] }) };
    expect(labels('- work\n- |', tags)).not.toContain('work');
  });

  it('filters on what is typed before the cursor, and replaces the whole value', () => {
    // CodeMirror filters on from–to, so `to` is the cursor: at the start of a value everything is offered.
    expect(labels('status: |draft')).toEqual(['draft', 'stable', 'deprecated']);
    expect(complete('status: dr|aft # x')!.to ?? 10).toBe(10);
    expect(applied('status: |draft # x', 'stable')).toBe('status: stable # x');
    expect(applied('status: dr|aft # x', 'draft')).toBe('status: draft # x');
    expect(applied('sta|tus: draft', 'status')).toBe('status: draft');
    expect(applied('tags: [wo|rk, family]', 'clients')).toBe('tags: [clients, family]');
  });

  it('in a comma string of tags, replaces only the tag at the cursor', () => {
    expect(applied('tags: fam|ily, friends', 'clients')).toBe('tags: clients, friends');
    expect(applied('tags: family, fr|', 'friends')).toBe('tags: family, friends');
  });

  it('opens on its own after a value’s space, not on a bare new line', () => {
    expect(complete('status: |', {}, true)).not.toBeNull();
    expect(complete('title: x\n|', {}, true)).toBeNull();
    expect(complete('title: x\nst|', {}, true)).not.toBeNull();
  });
});

describe('wikilink completions', () => {
  it('inside `[[`, every note by title, quoted to suit', () => {
    const c = complete('relationships:\n  - type: parent\n    to: [[Al|')!;
    expect(c.from).toBe('relationships:\n  - type: parent\n    to: '.length);
    const alice = c.options.find((o) => o.displayLabel === 'Alice Smith')!;
    expect(alice.label).toBe('[[Alice Smith]]');
    expect(alice.detail).toBe('/people/alice/');
    const apply = (src: string, name: string) => {
      const res = complete(src)!;
      const opt = res.options.find((o) => o.displayLabel === name)!;
      const pos = src.lastIndexOf('|');
      const doc = src.slice(0, pos) + src.slice(pos + 1);
      const view = new EditorView({ state: EditorState.create({ doc, extensions: [yaml()] }) });
      (opt.apply as (v: EditorView, c: Completion, f: number, t: number) => void)(view, opt, res.from, res.to ?? res.from);
      const out = view.state.doc.toString();
      view.destroy();
      return out;
    };
    expect(apply('to: [[Al|', 'Alice Smith')).toBe('to: "[[Alice Smith]]"');
    expect(apply('to: "[[Al|', 'Alice Smith')).toBe('to: "[[Alice Smith]]"');
    expect(apply('to: "[[Al|ice]]"', 'Alice Smith')).toBe('to: "[[Alice Smith]]"');
    expect(apply("to: '[[Al|'", 'Alice Smith')).toBe("to: '[[Alice Smith]]'");
    expect(apply('aliases: ["[[|"]', 'Say "hi"')).toBe('aliases: ["[[Say \\"hi\\"]]"]');
    expect(apply('summary: met [[Al|', 'Alice Smith')).toBe('summary: met [[Alice Smith]]');
  });
});

describe('yamlScalarText', () => {
  it('writes plain where that reads the same, quotes otherwise, escapes inside quotes', () => {
    expect(yamlScalarText('draft', { quote: null, flow: false })).toBe('draft');
    expect(yamlScalarText('Attested Computation', { quote: null, flow: false })).toBe('Attested Computation');
    expect(yamlScalarText('75010', { quote: null, flow: false })).toBe('"75010"');
    expect(yamlScalarText('a: b', { quote: null, flow: false })).toBe('"a: b"');
    expect(yamlScalarText('a, b', { quote: null, flow: true })).toBe('"a, b"');
    expect(yamlScalarText('a, b', { quote: null, flow: false })).toBe('a, b');
    expect(yamlScalarText('yes', { quote: null, flow: false })).toBe('yes');
    expect(yamlScalarText('true', { quote: null, flow: false })).toBe('"true"');
    expect(yamlScalarText('true', { quote: null, flow: false }, true)).toBe('true');
    expect(yamlScalarText('say "hi"', { quote: '"', flow: false })).toBe('say \\"hi\\"');
    expect(yamlScalarText("it's", { quote: "'", flow: false })).toBe("it''s");
  });
});

describe('yamlDiagnostics', () => {
  it('maps parse errors to ranges in the text', () => {
    const text = 'a: [1\nb: 2';
    const d = yamlDiagnostics(text);
    expect(d.length).toBeGreaterThan(0);
    expect(d[0].severity).toBe('error');
    expect(d[0].source).toBe('YAML');
    expect(d[0].message).not.toContain('\n');
    for (const x of d) {
      expect(x.from).toBeGreaterThanOrEqual(0);
      expect(x.to).toBeLessThanOrEqual(text.length);
      expect(x.to).toBeGreaterThan(x.from);
    }
  });

  it('marks the offending token', () => {
    const text = 'title: x\n  bad: indent\n';
    const [first] = yamlDiagnostics(text);
    expect(text.slice(first.from, first.to).trim()).not.toBe('');
  });

  it('has nothing to say about valid YAML, and handles an error at the very end', () => {
    expect(yamlDiagnostics('a: 1\nb: [x, y]\n')).toEqual([]);
    expect(yamlDiagnostics('')).toEqual([]);
    const end = yamlDiagnostics('a: "unclosed');
    expect(end.length).toBeGreaterThan(0);
    expect(end.every((x) => x.to <= 'a: "unclosed'.length && x.to > x.from)).toBe(true);
  });

  it('marks the end of the line, not the line break, for an unclosed quote', () => {
    const text = 'a: "x\nb: 1\n';
    const [d] = yamlDiagnostics(text);
    expect(text.slice(d.from, d.to)).not.toMatch(/^\n/);
    expect(d.to - d.from).toBeGreaterThan(0);
  });

  it('an alias with no anchor is an error at the alias', () => {
    const text = 'a: 1\nb: *nope\n';
    const [d] = yamlDiagnostics(text);
    expect(d.message).toMatch(/\*nope/);
    expect(text.slice(d.from, d.to)).toBe('*nope');
  });

  it('warns about duplicate keys as errors (the model refuses them)', () => {
    expect(yamlDiagnostics('a: 1\na: 2\n').map((x) => x.severity)).toContain('error');
  });
});

/** CodeMirror's `Mod`: Cmd on a Mac, Ctrl elsewhere (happy-dom's platform decides). */
const MOD: KeyboardEventInit = /Mac/.test(navigator.platform) ? { metaKey: true } : { ctrlKey: true };

describe('the editor’s keys inside the editor modal', () => {
  let editor: YamlSourceEditor | null = null;
  const seen: string[] = [];
  const onDoc = (e: KeyboardEvent) => seen.push(`${e.metaKey ? 'Mod-' : ''}${e.key}`);

  afterEach(() => {
    editor?.destroy();
    editor = null;
    seen.length = 0;
    document.removeEventListener('keydown', onDoc);
    document.body.innerHTML = '';
  });

  const mountEditor = (extra: Partial<Parameters<typeof createYamlEditor>[0]> = {}) => {
    const parent = document.createElement('div');
    document.body.appendChild(parent);
    document.addEventListener('keydown', onDoc);
    const changes: string[] = [];
    editor = createYamlEditor({ parent, doc: 'status: ', label: 'YAML', knowledge: () => knowledge(), onChange: (t) => changes.push(t), ...extra });
    return { view: editor.view, changes };
  };
  const key = (view: EditorView, init: KeyboardEventInit) => {
    const ev = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
    view.contentDOM.dispatchEvent(ev);
    return ev;
  };

  it('Escape with nothing open reaches the modal (closing it), as from the old textarea', () => {
    const { view } = mountEditor();
    key(view, { key: 'Escape' });
    expect(seen).toEqual(['Escape']);
  });

  it('Escape that closes the completion list stops there', async () => {
    const { view } = mountEditor();
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    startCompletion(view);
    for (let i = 0; i < 20 && completionStatus(view.state) !== 'active'; i++) await new Promise((r) => setTimeout(r, 10));
    expect(completionStatus(view.state)).toBe('active');
    const ev = key(view, { key: 'Escape' });
    expect(ev.defaultPrevented).toBe(true);
    expect(completionStatus(view.state)).toBeNull();
    expect(seen).toEqual([]);
  });

  it('a slice’s Escape cancels it and Mod-Enter applies it, neither reaching the modal', () => {
    let escaped = 0;
    let submitted = 0;
    const { view } = mountEditor({ onEscape: () => escaped++, onSubmit: () => submitted++ });
    key(view, { key: 'Escape' });
    key(view, { key: 'Enter', ...MOD });
    expect([escaped, submitted]).toEqual([1, 1]);
    expect(seen).toEqual([]);
  });

  it('Cmd/Ctrl+S reaches the modal’s save and inserts nothing', () => {
    const { view, changes } = mountEditor();
    key(view, { key: 's', ...MOD });
    expect(seen).toEqual([MOD.metaKey ? 'Mod-s' : 's']);
    expect(view.state.doc.toString()).toBe('status: ');
    expect(changes).toEqual([]);
  });

  it('setText replaces the text without reporting it; edits are reported', () => {
    const { view, changes } = mountEditor();
    editor!.setText('a: 1\n');
    expect(changes).toEqual([]);
    view.dispatch({ changes: { from: 5, insert: 'b: 2\n' }, userEvent: 'input.type' });
    expect(changes).toEqual(['a: 1\nb: 2\n']);
    expect(editor!.text).toBe('a: 1\nb: 2\n');
  });
});
