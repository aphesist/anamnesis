// Settings sections that talk to the server: connections (models, keys, "Continue with
// ChatGPT", the claude cli, routing, connecting a harness over mcp), sources, and
// privacy (the default for new items and the egress ledger, "what left this machine").
// Mounted in the settings sheet by main.js, next to ui/theme's appearance settings.
import { h, clear, fmtDate } from '../lib/dom.js';
import { api } from '../lib/api.js';
import { patchSettings } from '../lib/settings.js';
import { normKind, isLocal, egressLabel, displayLabel, keepsCase, takesKey, access, termsGrey, keyStorage } from '../lib/backends.js';
import { renderSchemaForm, collect } from '../lib/schemaform.js';

const sect = (title, ...kids) => h('section.cx-sect', h('h3.cx-h', title), ...kids);
const meta = (...t) => h('p.meta', ...t);
const changed = () => window.dispatchEvent(new CustomEvent('anamnesis:backendschange'));
const errText = (e) => (e && (e.serverMessage || e.message)) || 'something went wrong';

// The harness's notice for the installed claude cli, shown before it can be enabled when
// the server does not send one (the backend row carries it once detection has run).
const CLAUDE_CLI_NOTICE = "uses your own claude login through anthropic's official cli. anthropic has not explicitly approved third-party apps doing this and could restrict it at any time; the safest routes are the mcp server and an api key. anamnesis never reads your credentials.";

const KIND_WORDS = {
  openai_compatible: 'openai-compatible endpoint',
  anthropic: 'anthropic api key',
  gemini: 'gemini api key',
  chatgpt: 'chatgpt plan',
  claude_cli: 'installed claude cli',
  retrieval: 'built in',
  other: 'other',
};
export const PRIVACY = [['local_only', 'local only'], ['cloud_ok', 'cloud ok'], ['ask', 'ask']];

function nameOf(b) { return h(keepsCase(b) ? 'span.brand-label' : 'span', displayLabel(b)); }

function choiceRow(label, options, value, onPick) {
  const g = h('div.choice', { role: 'radiogroup', 'aria-label': label });
  for (const [v, text] of options) {
    const b = h('button', { type: 'button', role: 'radio', 'aria-checked': String(v === value), onclick: async () => {
      for (const x of g.children) x.setAttribute('aria-checked', String(x === b));
      await onPick(v);
    } }, text);
    g.append(b);
  }
  return g;
}

// ---------------------------------------------------------------------------------
// connections

export async function mountConnections(el, ctx) {
  el.classList.add('cx');
  const status = h('p.meta', 'reading…');
  el.append(status);
  let resp, settings, chatgpt = null;
  try {
    [resp, settings] = await Promise.all([api.backends(), api.get('/api/settings').catch(() => ({}))]);
  } catch (e) { status.textContent = 'the server could not be reached: ' + errText(e); return; }
  try { chatgpt = await api.get('/api/backends/chatgpt'); } catch { chatgpt = null; }
  const reload = async () => { clear(el); changed(); await mountConnections(el, ctx); };
  const backends = resp.backends || [];
  status.remove();

  // ----- models
  const rows = h('ul.list.cx-backends');
  for (const b of backends) rows.append(backendRow(b));
  const storage = keyStorage(null, resp) || backends.map((b) => keyStorage(b, resp)).find(Boolean);
  el.append(sect('models',
    meta(resp.detected === false ? 'detection has not run (the server was started without it). press refresh to look for local models and keys.' : 'what anamnesis can call, and where each call goes.'),
    rows,
    h('div.btnrow', h('button.textbtn', { type: 'button', onclick: async (e) => { e.target.textContent = 'looking…'; try { await api.post('/api/backends/refresh'); } catch (er) { ctx.toast(errText(er)); } reload(); } }, 'refresh'),
      h('span.meta', storage ? `keys are kept in the ${storage}; they are never shown again.` : 'keys go to the os keychain; they are never shown again.')),
    addProvider()));

  function backendRow(b) {
    const local = isLocal(b, settings);
    const grey = termsGrey(b);
    const li = h('li.cx-backend', { 'data-kind': normKind(b.kind) },
      h('div.cx-line', nameOf(b), h('span.meta', ` · ${KIND_WORDS[normKind(b.kind)] || b.kind}`)),
      h('div.cx-tags',
        h(`span.tag.${b.available === false ? 'off' : 'on'}`, b.status || (b.available === false ? 'unavailable' : 'ready')),
        h(`span.tag.where.${local ? 'here' : 'away'}`, egressLabel(b, settings)),
        h(`span.tag.terms.${grey ? 'grey' : 'ok'}`, grey ? 'terms: grey' : 'terms: allowed')));
    if (grey && b.notice) li.append(h('p.notice', b.notice));
    else if (b.notice && normKind(b.kind) !== 'chatgpt') li.append(meta(b.notice));
    if (takesKey(b, settings)) li.append(keyEntry(b.id, b));
    if (normKind(b.kind) === 'openai_compatible' && (settings?.harness?.openai_compatible || []).some((c) => c.id === b.id)) {
      li.append(h('div.btnrow', h('button.textbtn', { type: 'button', onclick: async () => {
        const list = (settings.harness.openai_compatible || []).filter((c) => c.id !== b.id);
        try { await patchSettings(api, { harness: { openai_compatible: list } }); ctx.toast('removed.'); reload(); } catch (e) { ctx.toast(errText(e)); }
      } }, 'remove this endpoint')));
    }
    return li;
  }

  function keyEntry(account, b) {
    const input = h('input.field', { type: 'password', placeholder: 'paste a key', autocomplete: 'off', spellcheck: 'false', 'aria-label': `api key for ${account}` });
    const msg = h('span.meta.cx-keymsg', { 'aria-live': 'polite' });
    const save = h('button.textbtn.strong', { type: 'button', onclick: async () => {
      const key = input.value.trim();
      if (!key) { input.focus(); return; }
      save.disabled = true;
      try { await api.put(`/api/backends/${encodeURIComponent(account)}/key`, { key }); input.value = ''; msg.textContent = 'saved. checking it…'; setTimeout(reload, 1800); }
      catch (e) { msg.textContent = errText(e); }
      finally { save.disabled = false; input.value = ''; }
    } }, 'save key');
    const forget = h('button.textbtn', { type: 'button', onclick: async () => {
      try { await api.del(`/api/backends/${encodeURIComponent(account)}/key`); msg.textContent = 'forgotten.'; setTimeout(reload, 1200); }
      catch (e) { msg.textContent = errText(e); }
    } }, 'forget key');
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') save.click(); });
    const st = b ? keyStorage(b, resp) : null;
    return h('div.cx-key', input, save, forget, st ? h('span.meta.cx-store', st) : null, msg);
  }

  function addProvider() {
    const has = (k) => backends.some((b) => normKind(b.kind) === k) || settings?.harness?.[k];
    const box = h('details.cx-add', h('summary', 'add a model'));
    const form = h('div.cx-addform');
    const pick = choiceRow('kind', [['anthropic', 'anthropic'], ['gemini', 'gemini'], ['openai_compatible', 'openai-compatible (ollama, openrouter, …)']], null, (v) => draw(v));
    const draw = (kind) => {
      clear(form);
      if (kind === 'anthropic' || kind === 'gemini') {
        if (has(kind)) { form.append(meta(`${kind} is already set up above; its key can be changed there.`)); return; }
        const enable = h('button.textbtn.strong', { type: 'button', onclick: async () => {
          try { await patchSettings(api, { harness: { [kind]: {} } }); ctx.toast(`${kind} added. paste its key.`); reload(); } catch (e) { ctx.toast(errText(e)); }
        } }, `add ${kind}`);
        form.append(meta(`calls to ${kind} leave this machine and are logged. the key is stored separately from settings.`), keyEntry(kind), h('div.btnrow', enable));
        return;
      }
      const f = {
        label: h('input.field', { type: 'text', placeholder: 'my server', 'aria-label': 'label' }),
        provider: h('input.field', { type: 'text', placeholder: 'ollama, openrouter, openai…', 'aria-label': 'provider' }),
        base_url: h('input.field', { type: 'text', placeholder: 'http://127.0.0.1:11434/v1', 'aria-label': 'base url' }),
        model: h('input.field', { type: 'text', placeholder: 'model id', 'aria-label': 'model' }),
      };
      const msg = h('span.meta', { 'aria-live': 'polite' });
      const add = h('button.textbtn.strong', { type: 'button', onclick: async () => {
        const base = f.base_url.value.trim(), model = f.model.value.trim();
        if (!/^https?:\/\//.test(base)) { msg.textContent = 'the base url should start with http:// or https://'; f.base_url.focus(); return; }
        if (!model) { msg.textContent = 'which model?'; f.model.focus(); return; }
        const provider = f.provider.value.trim().toLowerCase() || 'openai-compatible';
        const id = (provider + '-' + model).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 48);
        const list = [...(settings?.harness?.openai_compatible || []).filter((c) => c.id !== id), { id, provider, base_url: base, model, ...(f.label.value.trim() ? { label: f.label.value.trim() } : {}) }];
        try { await patchSettings(api, { harness: { openai_compatible: list } }); ctx.toast('added. if it needs a key, paste it on its row.'); reload(); } catch (e) { msg.textContent = errText(e); }
      } }, 'add endpoint');
      form.append(...Object.entries(f).map(([k, i]) => h('div.form-row', h('label', k.replace('_', ' ')), i)), h('div.btnrow', add, msg),
        meta('loopback addresses stay on this machine; anything else leaves it and is logged.'));
    };
    box.append(pick, form);
    return box;
  }

  // ----- continue with chatgpt
  if (chatgpt) el.append(sect('chatgpt', chatgptBlock(chatgpt)));
  function chatgptBlock(info) {
    const wrap = h('div.cx-chatgpt');
    const draw = (info) => {
      clear(wrap);
      const st = info.status || {};
      if (st.signed_in) {
        wrap.append(
          h('p', 'signed in', st.email ? h('span.keep-case', ` as ${st.email}`) : '', st.plan_usage ? h('span.meta', ` · ${typeof st.plan_usage === 'string' ? st.plan_usage : 'on your plan\'s usage'}`) : ''),
          h('div.btnrow', h('button.textbtn', { type: 'button', onclick: async () => { try { await api.post('/api/backends/chatgpt/sign-out'); ctx.toast('signed out here. revoke access in chatgpt settings if you want it gone there too.'); reload(); } catch (e) { ctx.toast(errText(e)); } } }, 'sign out'),
            info.usage_settings_url ? h('a', { href: info.usage_settings_url, target: '_blank', rel: 'noopener' }, 'usage settings') : null));
      } else {
        const btn = h('button.cx-signin.brand-label', { type: 'button', onclick: () => signIn(btn) }, info.button_label || 'Continue with ChatGPT');
        wrap.append(btn);
        if (info.sign_in?.in_progress) wrap.append(meta('waiting for the browser…'));
        if (info.sign_in?.last_error) wrap.append(h('p.notice', info.sign_in.last_error));
        if (st.message) wrap.append(meta(st.message));
      }
      if (info.notice) wrap.append(h('p.notice', info.notice));
    };
    const signIn = async (btn) => {
      btn.disabled = true;
      let r;
      try { r = await api.post('/api/backends/chatgpt/sign-in'); } catch (e) { btn.disabled = false; ctx.toast(errText(e)); return; }
      if (r.authorize_url) window.open(r.authorize_url, '_blank', 'noopener');
      wrap.append(meta('a browser tab opened for the sign-in. this page will notice when it is done.'));
      const started = Date.now();
      const poll = async () => {
        let info;
        try { info = await api.get('/api/backends/chatgpt'); } catch { info = null; }
        if (info && (info.status?.signed_in || (!info.sign_in?.in_progress && Date.now() - started > 2500))) { draw(info); if (info.status?.signed_in) { ctx.toast('signed in.'); changed(); } return; }
        if (Date.now() - started < 5 * 60e3 && wrap.isConnected) setTimeout(poll, 2000);
        else if (info) draw(info);
      };
      setTimeout(poll, 2000);
    };
    draw(info);
    return wrap;
  }

  // ----- the installed claude cli
  {
    const cliRow = backends.find((b) => normKind(b.kind) === 'claude_cli');
    const enabled = !!settings?.harness?.claude_cli?.enabled;
    const notice = cliRow?.notice || CLAUDE_CLI_NOTICE;
    const wrap = h('div.cx-cli');
    const draw = (askFirst) => {
      clear(wrap);
      const t = h('button.toggle', { type: 'button', role: 'switch', 'aria-checked': String(enabled) }, h('span.tick', { 'aria-hidden': 'true' }), h('span', 'use my installed claude cli'));
      t.addEventListener('click', async () => {
        if (!enabled && !askFirst) { draw(true); return; }
        if (enabled) { try { await patchSettings(api, { harness: { claude_cli: { enabled: false } } }); ctx.toast('the claude cli is off.'); setTimeout(reload, 600); } catch (e) { ctx.toast(errText(e)); } }
      });
      wrap.append(t, meta(cliRow ? cliRow.status || '' : enabled ? 'on. it is used once detection finds the cli on this machine.' : 'off.'));
      if (askFirst || enabled) wrap.append(h('p.notice', notice));
      if (askFirst && !enabled) wrap.append(h('div.btnrow',
        h('button.textbtn.strong', { type: 'button', onclick: async () => { try { await patchSettings(api, { harness: { claude_cli: { enabled: true } } }); ctx.toast('enabled, at your request.'); setTimeout(reload, 900); } catch (e) { ctx.toast(errText(e)); } } }, 'enable it, at my request'),
        h('button.textbtn', { type: 'button', onclick: () => draw(false) }, 'not now')));
    };
    draw(false);
    el.append(sect('claude cli', wrap));

    // optional, grey area: relabel requests as anamnesis (off by default; notice first)
    if (enabled && resp.claude_cli_relabel_offered !== false) {
      const relabel = !!settings?.harness?.claude_cli?.relabel_traffic;
      const rnotice = resp.notices?.claude_cli_relabel || '';
      const rwrap = h('div.cx-cli');
      const rdraw = (askFirst) => {
        clear(rwrap);
        const t = h('button.toggle', { type: 'button', role: 'switch', 'aria-checked': String(relabel) }, h('span.tick', { 'aria-hidden': 'true' }), h('span', 'label requests as anamnesis'));
        t.addEventListener('click', async () => {
          if (!relabel && !askFirst) { rdraw(true); return; }
          if (relabel) { try { await patchSettings(api, { harness: { claude_cli: { relabel_traffic: false } } }); ctx.toast('requests use the cli\'s own label again.'); setTimeout(reload, 600); } catch (e) { ctx.toast(errText(e)); } }
        });
        rwrap.append(t, meta(relabel ? 'on. grey area, your account only.' : 'off.'));
        if (askFirst || relabel) rwrap.append(h('p.notice', rnotice));
        if (askFirst && !relabel) rwrap.append(h('div.btnrow',
          h('button.textbtn.strong', { type: 'button', onclick: async () => { try { await patchSettings(api, { harness: { claude_cli: { relabel_traffic: true } } }); ctx.toast('relabelling on, at your request.'); setTimeout(reload, 900); } catch (e) { ctx.toast(errText(e)); } } }, 'i understand the risk, turn it on'),
          h('button.textbtn', { type: 'button', onclick: () => rdraw(false) }, 'not now')));
      };
      rdraw(false);
      el.append(sect('claude cli · relabel', rwrap));
    }
  }

  // ----- routing
  {
    const tasks = [['chat', 'chat'], ['label', 'naming regions'], ['extract', 'finding threads'], ['summarise', 'summaries']];
    const chosenRouting = settings?.routing || {};
    const usable = backends.filter((b) => b.available !== false);
    const grid = h('div.cx-routing');
    for (const [task, words] of tasks) {
      const sel = h('select.quiet', { 'aria-label': `model for ${words}` },
        h('option', { value: '' }, `automatic${resp.routing?.[task] ? ' (' + resp.routing[task] + ')' : ''}`),
        ...usable.filter((b) => task === 'chat' || normKind(b.kind) !== 'retrieval').map((b) => h('option', { value: b.id, selected: chosenRouting[task] === b.id }, `${displayLabel(b)} · ${isLocal(b, settings) ? 'here' : 'leaves'}`)));
      sel.addEventListener('change', async () => {
        try { await patchSettings(api, { routing: { [task]: sel.value || null } }); ctx.toast(`${words}: ${sel.value || 'automatic'}.`); changed(); } catch (e) { ctx.toast(errText(e)); }
      });
      grid.append(h('label', words), sel);
    }
    el.append(sect('routing', meta('which model does which job. background jobs never run on a subscription plan.'), grid));
  }

  // ----- connect a harness (mcp)
  el.append(sect('connect a harness', await harnessBlock(ctx)));
}

async function harnessBlock(ctx) {
  const wrap = h('div.cx-mcp');
  let r;
  try { r = await api.get('/api/mcp/snippets'); } catch (e) { wrap.append(meta('mcp snippets unavailable: ' + errText(e))); return wrap; }
  const s = r.snippets || {};
  const items = [
    ['claude code', s.claude_code_command, 'run in a terminal'],
    ['claude code (http)', s.claude_code_http_command, 'with the token in the environment'],
    ['claude code · .mcp.json', s.claude_code_mcp_json],
    ['claude desktop', s.claude_desktop_json, 'claude_desktop_config.json'],
    ['codex', s.codex_toml, '~/.codex/config.toml'],
    ['codex (http)', s.codex_toml_http],
    ['gemini cli', s.gemini_settings_json, '~/.gemini/settings.json'],
    ['gemini cli (http)', s.gemini_settings_json_http],
  ].filter(([, v]) => v);
  wrap.append(meta('external harnesses can read and write this brain over mcp. paste one of these into the client you use.'));
  const pick = h('div.choice', { role: 'tablist', 'aria-label': 'client' });
  const pane = h('div.cx-snippet');
  const show = (k) => {
    for (const b of pick.children) b.setAttribute('aria-selected', String(+b.dataset.k === k));
    const [name, text, where] = items[k];
    clear(pane).append(h('pre.keep-case', { tabindex: '0', 'aria-label': `${name} configuration` }, text), h('div.btnrow', copyBtn(text, ctx), where ? h('span.meta.keep-case', where) : null));
  };
  items.forEach(([name], k) => pick.append(h('button', { type: 'button', role: 'tab', 'data-k': String(k), onclick: () => show(k) }, name)));
  wrap.append(pick, pane);
  if (items.length) show(0);
  // the token: hidden until asked
  if (r.token) {
    const tok = h('code.keep-case.cx-token', '•'.repeat(24));
    let shown = false;
    const reveal = h('button.textbtn', { type: 'button', 'aria-pressed': 'false', onclick: () => { shown = !shown; tok.textContent = shown ? r.token : '•'.repeat(24); reveal.textContent = shown ? 'hide' : 'reveal'; reveal.setAttribute('aria-pressed', String(shown)); } }, 'reveal');
    wrap.append(...[h('div.cx-tokrow', h('span.meta', `http token (set ${r.token_env_var || 'ANAMNESIS_MCP_TOKEN'}):`), tok, h('div.btnrow', reveal, copyBtn(r.token, ctx, 'copy token'))),
      r.http_url ? meta('http endpoint: ', h('span.keep-case', r.http_url)) : null].filter(Boolean));
  }
  if (r.tools?.length) wrap.append(meta(`${r.tools.length} tools: `, h('span.keep-case', r.tools.join(', '))));
  return wrap;
}

function copyBtn(text, ctx, label = 'copy') {
  const b = h('button.textbtn', { type: 'button', onclick: async () => {
    try { await navigator.clipboard.writeText(text); b.textContent = 'copied'; setTimeout(() => { b.textContent = label; }, 1400); }
    catch { ctx.toast('the clipboard is not available here; select the text instead.'); }
  } }, label);
  return b;
}

// ---------------------------------------------------------------------------------
// sources

export async function mountSources(el, ctx) {
  el.classList.add('cx');
  const status = h('p.meta', 'reading…');
  el.append(status);
  let r;
  try { r = await api.get('/api/sources'); } catch (e) { status.textContent = 'sources unavailable: ' + errText(e); return; }
  status.remove();
  const reload = () => { clear(el); mountSources(el, ctx); };
  const connectors = r.connectors || [];
  const list = h('ul.list.cx-sources');
  const sources = r.sources || [];
  if (!sources.length) list.append(h('li.empty', 'no sources yet. add a folder, a vault or a feed below.'));
  for (const s of sources) {
    const out = h('span.meta', { 'aria-live': 'polite' });
    const internal = s.connector === 'editor' || s.connector === 'chat' || s.connector === 'demo';
    const priv = h('select.quiet', { 'aria-label': `default privacy for ${s.name}` }, ...PRIVACY.map(([v, l]) => h('option', { value: v, selected: s.default_privacy === v }, l)));
    priv.addEventListener('change', async () => {
      try { await api.put(`/api/sources/${encodeURIComponent(s.id)}`, { default_privacy: priv.value }); out.textContent = 'new items from here: ' + priv.selectedOptions[0].textContent + '.'; }
      catch (e) { out.textContent = errText(e); }
    });
    const sync = h('button.textbtn', { type: 'button', onclick: async () => {
      sync.disabled = true; out.textContent = 'reading…';
      try {
        const x = await api.post(`/api/sources/${encodeURIComponent(s.id)}/sync`);
        out.textContent = ['created', 'updated', 'unchanged', 'deleted', 'renamed'].filter((k) => x[k]).map((k) => `${x[k]} ${k}`).join(' · ') || 'nothing new.';
        if (x.created || x.updated || x.deleted) ctx.loadCloud?.().catch(() => {});
      } catch (e) { out.textContent = errText(e); }
      sync.disabled = false;
    } }, 'sync now');
    const remove = h('button.textbtn', { type: 'button', onclick: () => {
      if (remove.dataset.armed !== '1') { remove.dataset.armed = '1'; remove.textContent = 'remove it and its items?'; setTimeout(() => { remove.dataset.armed = ''; remove.textContent = 'remove'; }, 4000); return; }
      api.del(`/api/sources/${encodeURIComponent(s.id)}`).then((x) => { ctx.toast(`removed. ${x.items_deleted ?? 0} items went with it.`); ctx.loadCloud?.().catch(() => {}); reload(); }).catch((e) => { out.textContent = errText(e); });
    } }, 'remove');
    const where = s.config?.path || s.config?.url || s.config?.feed || '';
    list.append(h('li.cx-source',
      h('div.cx-line', h('span.keep-case', s.name), h('span.meta', ` · ${s.connector}`)),
      where ? h('div.meta.keep-case.cx-where', String(where)) : null,
      h('div.btnrow', internal ? null : sync, h('label.meta', 'new items: ', priv), internal ? null : remove, out)));
  }
  el.append(sect('sources', list));

  // add a source from a connector manifest
  const addable = connectors.filter((c) => !['editor', 'chat', 'demo', 'exec'].includes(c.id));
  if (addable.length) {
    const form = h('div.cx-addform');
    const pick = choiceRow('connector', addable.map((c) => [c.id, (c.name || c.id).toLowerCase()]), null, (id) => draw(addable.find((c) => c.id === id)));
    const draw = (m) => {
      clear(form);
      const sf = renderSchemaForm(m.config_schema || {}, { idPrefix: 'src-' + m.id });
      const name = h('input.field', { type: 'text', placeholder: m.name || m.id, 'aria-label': 'name' });
      const priv = h('select.quiet', { 'aria-label': 'privacy for its items' }, ...PRIVACY.map(([v, l]) => h('option', { value: v, selected: (m.default_privacy || 'cloud_ok') === v }, l)));
      const msg = h('span.meta', { 'aria-live': 'polite' });
      const go = h('button.textbtn.strong', { type: 'button', onclick: async () => {
        const { config, errors } = collect(m.config_schema || {}, sf.read());
        sf.showErrors(errors);
        if (Object.keys(errors).length) return;
        go.disabled = true; msg.textContent = 'checking…';
        try {
          await api.post('/api/sources', { connector: m.id, name: name.value.trim() || undefined, config, default_privacy: priv.value });
          ctx.toast('added. its items will appear as they are read.');
          reload();
        } catch (e) { msg.textContent = errText(e); go.disabled = false; }
      } }, 'add source');
      form.append(...[
        m.description ? meta(m.description) : null,
        h('div.form-row', h('label', 'name'), name),
        sf.el,
        h('div.form-row', h('label', 'privacy for its items'), priv),
        m.network_hosts?.length ? meta('contacts: ', h('span.keep-case', m.network_hosts.join(', '))) : null,
        h('div.btnrow', go, msg)].filter(Boolean));
    };
    el.append(sect('add a source', pick, form));
  }
}

// ---------------------------------------------------------------------------------
// privacy and the egress ledger

export async function mountPrivacy(el, ctx) {
  el.classList.add('cx');
  let settings = {};
  try { settings = await api.get('/api/settings'); } catch { settings = {}; }
  const cur = settings?.privacy?.default || 'cloud_ok';
  el.append(sect('new items',
    meta('the privacy a new item gets unless its source says otherwise. local only never leaves; ask waits for you in the chat.'),
    choiceRow('default privacy', PRIVACY, cur, async (v) => { try { await patchSettings(api, { privacy: { default: v } }); ctx.toast('saved.'); } catch (e) { ctx.toast(errText(e)); } })));
  const ledger = h('div.cx-ledger', h('p.meta', 'reading…'));
  el.append(sect('what left this machine', meta('every call to a model that is not local, newest first. counts only: the text itself is never logged.'), ledger));
  try {
    const r = await api.egress(200);
    renderLedger(ledger, r.entries || []);
  } catch (e) { clear(ledger).append(meta('the ledger could not be read: ' + errText(e))); }
}

// One egress event: { seq, at, actor, kind: { type: 'egress', provider, model, purpose, items, chars } }
// (also accepts a flattened row).
export function egressRow(e) {
  const k = e.kind && typeof e.kind === 'object' ? e.kind : e;
  return {
    at: e.at || k.at,
    provider: k.provider || '',
    model: k.model || '',
    purpose: k.purpose || '',
    items: Array.isArray(k.items) ? k.items.length : (k.items ?? 0),
    chars: k.chars ?? 0,
  };
}

function renderLedger(el, entries) {
  clear(el);
  if (!entries.length) { el.append(h('p.empty', 'nothing has left this machine.')); return; }
  const rows = entries.map(egressRow);
  const tot = rows.reduce((a, r) => ({ n: a.n + 1, items: a.items + r.items, chars: a.chars + r.chars }), { n: 0, items: 0, chars: 0 });
  el.append(h('p.meta', `${tot.n} calls · ${tot.items} items · ${tot.chars.toLocaleString('en')} characters`),
    h('table.tbl.cx-egress',
      h('thead', h('tr', ...['when', 'provider', 'model', 'purpose', 'items', 'chars'].map((t, k) => h(k > 3 ? 'th.num' : 'th', t)))),
      h('tbody', ...rows.map((r) => h('tr',
        h('td.data', fmtDate(r.at, true)), h('td', r.provider), h('td.keep-case.data.model', r.model), h('td', r.purpose),
        h('td.data.num', String(r.items)), h('td.data.num', r.chars.toLocaleString('en')))))));
}
