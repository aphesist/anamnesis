// Forms generated from a connector manifest's `config_schema` (a json schema object).
// Supports what the built-in connectors use: string (with pattern / minLength), integer,
// number, boolean, enum, arrays of strings (comma or newline separated) and nested
// objects. `collect` is pure, so the coercion and validation are unit-tested in node.
import { h } from './dom.js';

// Flatten the schema to fields with dotted paths. Required fields come first.
export function fieldsOf(schema, prefix = '', parentRequired = true) {
  const props = schema?.properties || {};
  const req = new Set(schema?.required || []);
  const out = [];
  for (const [name, s] of Object.entries(props)) {
    const path = prefix ? `${prefix}.${name}` : name;
    const required = parentRequired && req.has(name);
    if (s && s.type === 'object' && s.properties) out.push(...fieldsOf(s, path, required));
    else out.push({ path, name, schema: s || {}, required, type: typeOf(s) });
  }
  return out.sort((a, b) => Number(b.required) - Number(a.required));
}

export function typeOf(s) {
  if (!s) return 'string';
  if (Array.isArray(s.enum)) return 'enum';
  if (s.type === 'array') return 'list';
  if (s.type === 'boolean' || s.type === 'integer' || s.type === 'number') return s.type;
  return 'string';
}

const setPath = (obj, path, v) => {
  const parts = path.split('.');
  let cur = obj;
  for (const p of parts.slice(0, -1)) cur = cur[p] ??= {};
  cur[parts.at(-1)] = v;
};

// raw: { path: string | boolean } as read from the form. Returns { config, errors: { path: msg } }.
export function collect(schema, raw) {
  const config = {};
  const errors = {};
  for (const f of fieldsOf(schema)) {
    const r = raw[f.path];
    const s = f.schema;
    if (f.type === 'boolean') { if (r === true) setPath(config, f.path, true); continue; }
    const text = typeof r === 'string' ? r.trim() : '';
    if (!text) { if (f.required) errors[f.path] = 'needed'; continue; }
    let v = text;
    if (f.type === 'integer' || f.type === 'number') {
      v = Number(text);
      if (!Number.isFinite(v) || (f.type === 'integer' && !Number.isInteger(v))) { errors[f.path] = f.type === 'integer' ? 'a whole number' : 'a number'; continue; }
      if (s.minimum != null && v < s.minimum) { errors[f.path] = `at least ${s.minimum}`; continue; }
      if (s.maximum != null && v > s.maximum) { errors[f.path] = `at most ${s.maximum}`; continue; }
    } else if (f.type === 'list') {
      v = text.split(/[\n,]/).map((x) => x.trim()).filter(Boolean);
    } else if (f.type === 'enum') {
      if (!s.enum.includes(v)) { errors[f.path] = 'pick one'; continue; }
    } else {
      if (s.minLength && v.length < s.minLength) { errors[f.path] = 'too short'; continue; }
      if (s.pattern) { try { if (!new RegExp(s.pattern).test(v)) { errors[f.path] = s.pattern.startsWith('^https?') ? 'an http(s) url' : 'not the expected form'; continue; } } catch { /* unknown regex dialect: let the server judge */ } }
    }
    setPath(config, f.path, v);
  }
  return { config, errors };
}

const words = (path) => path.replace(/[._]/g, ' ');

// Render the form. Returns { el, read(), showErrors(errors) }.
export function renderSchemaForm(schema, { idPrefix = 'cfg' } = {}) {
  const fields = fieldsOf(schema);
  const inputs = new Map();
  const errs = new Map();
  const row = (f) => {
    const id = `${idPrefix}-${f.path.replace(/\./g, '-')}`;
    let input;
    if (f.type === 'boolean') {
      input = h('input', { type: 'checkbox', id });
      if (f.schema.default === true) input.checked = true;
    } else if (f.type === 'enum') {
      input = h('select.quiet', { id }, h('option', { value: '' }, f.required ? 'choose…' : 'default'), ...f.schema.enum.map((v) => h('option', { value: v }, words(String(v)))));
    } else {
      const ph = f.type === 'list' ? 'comma separated' : f.schema.pattern?.startsWith('^https?') ? 'https://…' : f.name === 'path' ? '~/notes' : '';
      input = h('input.field', { type: f.type === 'integer' || f.type === 'number' ? 'number' : 'text', id, placeholder: ph, autocomplete: 'off', spellcheck: 'false' });
      if (f.schema.default != null) input.value = Array.isArray(f.schema.default) ? f.schema.default.join(', ') : String(f.schema.default);
    }
    inputs.set(f.path, input);
    const err = h('span.field-err', { 'aria-live': 'polite' });
    errs.set(f.path, err);
    const label = h('label', { for: id }, words(f.path), f.required ? h('span.req', { 'aria-hidden': 'true' }, ' ·') : null);
    const hint = f.schema.description ? h('span.meta', f.schema.description) : null;
    return f.type === 'boolean'
      ? h('div.form-row.check', input, label, hint, err)
      : h('div.form-row', label, input, hint, err);
  };
  const req = fields.filter((f) => f.required).map(row);
  const opt = fields.filter((f) => !f.required).map(row);
  const el = h('div.schema-form', ...req, opt.length ? h('details.more-opts', h('summary', `more options (${opt.length})`), ...opt) : null);
  return {
    el,
    read() {
      const raw = {};
      for (const [p, i] of inputs) raw[p] = i.type === 'checkbox' ? i.checked : i.value;
      return raw;
    },
    showErrors(errors) {
      for (const [p, e] of errs) e.textContent = errors[p] || '';
      const first = Object.keys(errors)[0];
      if (first) { const i = inputs.get(first); i.closest('details')?.setAttribute('open', ''); i.focus(); }
    },
  };
}
