// Write: a plain textarea markdown editor with autosave (POST then PUT /api/items).
// Drafts are also kept in localStorage, so nothing is lost if the server is away.
import { h } from '../lib/dom.js';
import { api } from '../lib/api.js';
import { renderMarkdown } from '../lib/md.js';

const KEY = 'anamnesis.draft.v1';
const titleFrom = (body) => (body.split('\n').find((l) => l.trim()) || 'untitled').replace(/^#+\s*/, '').trim().slice(0, 120) || 'untitled';

export default {
  mount(el, ctx) {
    let draft = { id: null, body: '' };
    try { draft = { ...draft, ...JSON.parse(localStorage.getItem(KEY) || '{}') }; } catch { /* */ }
    const area = h('textarea.keep-case', { 'aria-label': 'note, in markdown', placeholder: '# a title\n\nwrite. it saves itself.', spellcheck: true });
    area.value = draft.body || '';
    const preview = h('div.preview-pane.reader-body', { hidden: true, 'aria-label': 'preview' });
    const wrapArea = h('div.write-area', area, preview);
    const status = h('span', { role: 'status', 'aria-live': 'polite' }, draft.id ? 'saved earlier' : 'not saved yet');
    const words = h('span');
    const prevBtn = h('button.textbtn', { type: 'button', 'aria-pressed': 'false' }, 'preview');
    const newBtn = h('button.textbtn', { type: 'button' }, 'new note');
    el.append(
      h('div.write-head', h('h2', 'write'), h('div.btnrow', prevBtn, newBtn)),
      wrapArea,
      h('div.write-foot', status, words, h('span', 'markdown · ⌘s saves now · your words, marked as written by you')));

    let timer = 0, saving = null, dirty = false;
    const keep = () => { try { localStorage.setItem(KEY, JSON.stringify(draft)); } catch { /* */ } };
    const count = () => { const n = (area.value.match(/\S+/g) || []).length; words.textContent = `${n} word${n === 1 ? '' : 's'}`; };
    const save = async () => {
      if (saving) { dirty = true; return saving; }
      const body = area.value;
      if (!body.trim()) return;
      dirty = false;
      status.textContent = 'saving…';
      const payload = { title: titleFrom(body), body };
      saving = (async () => {
        try {
          if (draft.id) await api.updateItem(draft.id, payload);
          else { const item = await api.createItem({ ...payload, kind: 'note' }); draft.id = item.id; }
          keep();
          const d = new Date();
          status.textContent = `saved ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
        } catch {
          status.textContent = 'not reachable — kept on this machine, will retry';
          clearTimeout(timer); timer = setTimeout(save, 8000);
        } finally {
          saving = null;
          if (dirty) save();
        }
      })();
      return saving;
    };
    area.addEventListener('input', () => {
      draft.body = area.value; keep(); count();
      status.textContent = 'editing…';
      if (!preview.hidden) preview.innerHTML = renderMarkdown(area.value);
      clearTimeout(timer); timer = setTimeout(save, 1200);
    });
    area.addEventListener('keydown', (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') { e.preventDefault(); clearTimeout(timer); save(); }
      // tab inserts two spaces; escape then tab leaves the field
      if (e.key === 'Tab' && !e.shiftKey && !area._escaped) { e.preventDefault(); area.setRangeText('  ', area.selectionStart, area.selectionEnd, 'end'); area.dispatchEvent(new Event('input')); }
      area._escaped = e.key === 'Escape';
    });
    area.addEventListener('blur', () => { if (draft.body.trim()) { clearTimeout(timer); save(); } });
    prevBtn.addEventListener('click', () => {
      preview.hidden = !preview.hidden;
      prevBtn.setAttribute('aria-pressed', String(!preview.hidden));
      wrapArea.classList.toggle('preview', !preview.hidden);
      if (!preview.hidden) preview.innerHTML = renderMarkdown(area.value);
    });
    newBtn.addEventListener('click', async () => {
      if (draft.body.trim()) await save();
      draft = { id: null, body: '' }; keep();
      area.value = ''; count(); status.textContent = 'a new page'; area.focus();
    });
    window.addEventListener('beforeunload', () => keep());
    count();
    this._focus = () => area.focus();
    void ctx;
  },
  shown() { setTimeout(() => this._focus?.(), 30); },
};
