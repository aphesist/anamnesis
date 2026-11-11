// A tiny fetch shim that answers /api/* from the fixtures in this folder,
// so the app runs with no server. Enabled with ?mock=1 (and ?n=20000 to
// inflate the cloud for a stress test). State changes (ratings, thread
// status, new notes) live in memory for the session only.

const BASE = new URL('./', import.meta.url);
const realFetch = window.fetch.bind(window);
const cache = new Map();
const load = (name) => {
  if (!cache.has(name)) cache.set(name, realFetch(new URL(name, BASE)).then((r) => r.json()));
  return cache.get(name);
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const json = (obj, status = 200) => new Response(JSON.stringify(obj), { status, headers: { 'content-type': 'application/json' } });
const iso = (sec) => new Date(sec * 1000).toISOString().replace(/\.\d{3}Z$/, 'Z');
const CROCK = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
const ulid = () => {
  let t = Date.now(), s = '';
  for (let i = 0; i < 10; i++) { s = CROCK[t % 32] + s; t = Math.floor(t / 32); }
  for (let i = 0; i < 16; i++) s += CROCK[(Math.random() * 32) | 0];
  return s;
};

const SOURCES = [
  { id: '01SRCEDITOR00000000000000A', connector: 'editor', name: 'the editor' },
  { id: '01SRCVAULT000000000000000B', connector: 'obsidian', name: 'journal vault', config: { path: '~/vaults/journal' } },
  { id: '01SRCRSS00000000000000000C', connector: 'rss', name: 'feeds', config: { url: 'https://example.org/feed.xml' } },
  { id: '01SRCPDF00000000000000000D', connector: 'pdf', name: 'papers folder', config: { path: '~/papers' } },
  { id: '01SRCCHAT0000000000000000E', connector: 'chat', name: 'anamnesis chats' },
  { id: '01SRCWEB00000000000000000F', connector: 'web', name: 'web clips', config: { url: 'https://example.org/essays' } },
];

// A few calls that left the machine, for the ledger (counts only, as the server logs them).
function seedEgress(c) {
  const now = Math.floor(Date.now() / 1000);
  const rows = [['anthropic', 'claude-sonnet-4-5', 'chat', 6, 5120], ['openrouter', 'meta-llama/llama-3.3-70b-instruct', 'label', 12, 3310], ['anthropic', 'claude-haiku-4-5', 'label', 9, 2408], ['anthropic', 'claude-sonnet-4-5', 'chat', 4, 3880]];
  return rows.map(([provider, model, purpose, n, chars], k) => ({
    seq: 900 - k * 37, at: iso(now - 3600 * (k * 19 + 2)), actor: { model: provider },
    kind: { type: 'egress', provider, model, purpose, items: c.ids.slice(k * 7, k * 7 + n), chars }, prev_hash: '', hash: '',
  }));
}

// items marked "ask" in the mock: every journal entry, and every eleventh item
const privacyOf = (s, i) => s.privacy.get(s.cloud.ids[i]) || (s.cloud.kinds[s.cloud.kind[i]] === 'journal' || i % 11 === 0 ? 'ask' : 'cloud_ok');

let state = null;
async function getState() {
  if (state) return state;
  const [cloud, extra, threads] = await Promise.all([load('cloud.json'), load('extra.json'), load('threads.json')]);
  const params = new URLSearchParams(location.search);
  const want = Math.min(60000, parseInt(params.get('n') || '0', 10) || 0);
  if (want > cloud.ids.length) inflate(cloud, want);
  const index = new Map(cloud.ids.map((id, i) => [id, i]));
  state = { cloud, extra, index, threads: threads.threads, created: new Map(), ratings: [], decided: new Map(), chats: 0,
    privacy: new Map(), links: [], labels: [], keys: new Set(['anthropic']), demo: false, chatgpt: false, egress: seedEgress(cloud),
    sources: SOURCES.map((x) => ({ ...x, config: x.config || {}, default_origin: x.connector === 'rss' || x.connector === 'web' ? 'external' : 'owner_written', default_privacy: x.connector === 'obsidian' ? 'ask' : 'cloud_ok', cursor: null, created_at: iso(cloud.t[0]) })),
    settings: { privacy: { default: 'cloud_ok' }, routing: { chat: 'ollama' }, harness: { openai_compatible: [{ id: 'openrouter', provider: 'openrouter', base_url: 'https://openrouter.ai/api/v1', model: 'meta-llama/llama-3.3-70b-instruct' }], anthropic: {}, claude_cli: { enabled: true } } } };
  return state;
}

// Duplicate points with jitter, for the stress test only.
function inflate(c, want) {
  const n0 = c.ids.length;
  for (let k = n0; k < want; k++) {
    const i = k % n0;
    c.ids.push(c.ids[i].slice(0, 20) + String(k).padStart(6, '0').slice(-6));
    for (let d = 0; d < 3; d++) c.xyz.push(c.xyz[i * 3 + d] + (Math.random() - 0.5) * 0.12);
    c.region.push(c.region[i]); c.kind.push(c.kind[i]); c.speaker.push(c.speaker[i]);
    c.t.push(c.t[i] + ((Math.random() * 3e6) | 0)); c.titles.push(c.titles[i]);
  }
}

function originOf(c, i) {
  const kind = c.kinds[c.kind[i]];
  const sp = c.speaker[i];
  if (sp === 1) return 'ai_generated';
  if (sp === 2) return 'external';
  if (kind === 'chat') return 'owner_in_chat';
  if (kind === 'quote') return 'owner_annotation';
  return 'owner_written';
}
function sourceOf(c, i) {
  const kind = c.kinds[c.kind[i]];
  if (kind === 'chat') return SOURCES[4];
  if (kind === 'feed') return SOURCES[2];
  if (kind === 'pdf' || kind === 'book' || kind === 'article') return SOURCES[3];
  if (kind === 'clip') return SOURCES[5];
  if (kind === 'journal') return SOURCES[1];
  return c.t[i] % 3 === 0 ? SOURCES[0] : SOURCES[1];
}

function bodyFor(s, i) {
  const c = s.cloud;
  const key = c.region[i] >= 0 ? s.extra.region_keys[c.region[i]] : 'unsorted';
  const base = s.extra.bodies[key];
  const title = c.titles[i];
  if (base && i % 4 === 0) return base;
  const reg = c.region[i] >= 0 ? c.regions[c.region[i]].label : 'nothing yet';
  return `${title.replace(/\.$/, '')}.\n\nwritten close to ${reg}. a short entry; the rest is in the margins.\n\n- what i noticed\n- what i want to remember\n\nsee also [[${c.titles[(i * 7 + 3) % c.ids.length]}]].`;
}

function itemAt(s, i) {
  const c = s.cloud;
  const src = sourceOf(c, i);
  const t = c.t[i];
  return {
    id: c.ids[i], source_id: src.id, external_id: `${src.connector}/${c.ids[i].slice(-8).toLowerCase()}.md`,
    uri: null, kind: c.kinds[c.kind[i]], title: c.titles[i], body: bodyFor(s, i),
    origin: originOf(c, i), privacy: privacyOf(s, i), created_at: iso(t), captured_at: iso(t + 3600 * ((i % 40) + 1)),
    modified_at: iso(t + 7200), content_hash: (c.ids[i] + c.ids[i]).toLowerCase().slice(0, 64),
    tags: [], meta: { source_name: src.name, connector: src.connector }, deleted: false,
  };
}

function linkObj(c, l) {
  return { from: c.ids[l[0]], to: c.ids[l[1]], kind: l[2], created_at: iso(c.t[l[0]]) };
}

function search(s, q, k = 20) {
  const c = s.cloud;
  const toks = q.toLowerCase().split(/[^a-z0-9']+/).filter((w) => w.length > 2);
  if (!toks.length) return [];
  const hits = [];
  for (let i = 0; i < c.ids.length; i++) {
    const tl = c.titles[i].toLowerCase();
    const rl = c.region[i] >= 0 ? c.regions[c.region[i]].label : '';
    let sc = 0;
    for (const w of toks) { if (tl.includes(w)) sc += 1; if (rl.includes(w)) sc += 0.35; }
    if (sc > 0) hits.push([sc + (c.t[i] % 97) / 1000, i]);
  }
  hits.sort((a, b) => b[0] - a[0]);
  return hits.slice(0, k).map(([score, i]) => ({
    item_id: c.ids[i], chunk_id: null, title: c.titles[i],
    snippet: c.titles[i] + ' — ' + (c.region[i] >= 0 ? c.regions[c.region[i]].label : 'unsorted'),
    score: +(score / toks.length).toFixed(3), start: 0, end: c.titles[i].length,
  }));
}

function nearest(s, i, k) {
  const c = s.cloud; const x = c.xyz;
  const linked = new Set(c.links.filter((l) => l[0] === i || l[1] === i).map((l) => (l[0] === i ? l[1] : l[0])));
  const d = [];
  for (let j = 0; j < c.ids.length; j++) {
    if (j === i || linked.has(j)) continue;
    const dx = x[j * 3] - x[i * 3], dy = x[j * 3 + 1] - x[i * 3 + 1], dz = x[j * 3 + 2] - x[i * 3 + 2];
    d.push([dx * dx + dy * dy + dz * dz, j]);
  }
  d.sort((a, b) => a[0] - b[0]);
  return d.slice(0, k);
}

function sse(events) {
  const enc = new TextEncoder();
  const body = new ReadableStream({
    async start(ctrl) {
      for (const [ev, data, wait] of events) {
        await sleep(wait || 0);
        ctrl.enqueue(enc.encode(`event: ${ev}\ndata: ${JSON.stringify(data)}\n\n`));
      }
      ctrl.close();
    },
  });
  return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } });
}

async function handle(method, url, body) {
  const s = await getState();
  const c = s.cloud;
  const p = url.pathname.replace(/^.*?\/api\//, '/api/');
  const q = url.searchParams;
  let m;
  await sleep(40 + Math.random() * 60);

  if (p === '/api/cloud') return json(c);
  if ((m = p.match(/^\/api\/items\/([^/]+)$/))) {
    const id = decodeURIComponent(m[1]);
    if (method === 'PUT') {
      if (body.privacy && !['local_only', 'cloud_ok', 'ask'].includes(body.privacy)) return json({ error: 'privacy: unknown variant' }, 400);
      const it = s.created.get(id);
      if (it) { Object.assign(it, body, { modified_at: iso(Date.now() / 1000) }); return json(it); }
      const i = s.index.get(id);
      if (i == null) return json({ error: 'not found' }, 404);
      if (body.privacy) s.privacy.set(id, body.privacy);
      return json(itemAt(s, i));
    }
    if (s.created.has(id)) return json({ item: s.created.get(id), links_out: [], links_in: [], threads: [], region: null });
    const i = s.index.get(id);
    if (i == null) return json({ error: 'not found' }, 404);
    return json({
      item: itemAt(s, i),
      links_out: [...s.links.filter((l) => l.from === id), ...c.links.filter((l) => l[0] === i).map((l) => linkObj(c, l))],
      links_in: c.links.filter((l) => l[1] === i).map((l) => linkObj(c, l)),
      threads: s.threads.filter((t) => t.item_id === id),
      region: c.region[i] >= 0 ? c.regions[c.region[i]] : null,
    });
  }
  if (p === '/api/items' && method === 'POST') {
    const now = iso(Date.now() / 1000);
    const it = {
      id: ulid(), source_id: SOURCES[0].id, external_id: 'editor/' + Date.now() + '.md', uri: null,
      kind: body.kind || 'note', title: body.title || 'untitled', body: body.body || '', origin: 'owner_written',
      privacy: 'cloud_ok', created_at: now, captured_at: now, modified_at: now, content_hash: '', tags: body.tags || [],
      meta: { source_name: 'the editor', connector: 'editor' }, deleted: false,
    };
    s.created.set(it.id, it);
    return json(it);
  }
  if (p === '/api/search') return json({ hits: search(s, q.get('q') || '', +(q.get('k') || 20)) });
  if (p === '/api/timeline') return json(await load('timeline.json'));
  if (p === '/api/threads' && method === 'GET') {
    let th = s.threads;
    if (q.get('kind')) th = th.filter((t) => t.kind === q.get('kind'));
    if (q.get('status')) th = th.filter((t) => t.status === q.get('status'));
    return json({ threads: th });
  }
  if ((m = p.match(/^\/api\/threads\/([^/]+)$/)) && method === 'POST') {
    const th = s.threads.find((t) => t.id === m[1]);
    if (!th) return json({ error: 'not found' }, 404);
    th.status = body.status; th.resolved_by = body.resolved_by ?? th.resolved_by; th.updated_at = iso(Date.now() / 1000);
    return json(th);
  }
  if (p === '/api/review' && method === 'GET') {
    return json({
      items: s.extra.review_idx.filter((i) => i < c.ids.length && !s.ratings.find((r) => r.i === i)).map((i, k) => ({
        resurface_id: 'RS' + c.ids[i].slice(2), item: itemAt(s, i), reason: s.extra.reasons[k % s.extra.reasons.length],
      })),
    });
  }
  if ((m = p.match(/^\/api\/review\/([^/]+)$/)) && method === 'POST') {
    const i = s.index.get('01' + m[1].slice(2));
    s.ratings.push({ id: m[1], i, useful: !!body.useful });
    return json({ ok: true });
  }
  if (p === '/api/suggestions') {
    const i = s.index.get(q.get('item'));
    if (i == null) return json({ suggestions: [] });
    return json({
      suggestions: nearest(s, i, 4).filter(([, j]) => !s.decided.has(c.ids[i] + c.ids[j])).map(([d, j]) => ({
        suggestion_id: 'SG' + c.ids[j].slice(2, 14) + c.ids[i].slice(14),
        to: { id: c.ids[j], title: c.titles[j], kind: c.kinds[c.kind[j]], created_at: iso(c.t[j]) },
        score: +(1 / (1 + 6 * Math.sqrt(d))).toFixed(3),
      })),
    });
  }
  if ((m = p.match(/^\/api\/suggestions\/([^/]+)$/)) && method === 'POST') {
    s.decided.set(m[1], !!body.accept);
    return json({ ok: true });
  }
  if (p === '/api/links' && method === 'POST') {
    if (!s.index.has(body.from) || !s.index.has(body.to)) return json({ error: 'no such item' }, 404);
    const l = { from: body.from, to: body.to, kind: 'manual', created_at: iso(Date.now() / 1000) };
    s.links.push(l);
    return json(l);
  }
  if (p === '/api/lineage') return json(await load('lineage.json'));
  if (p === '/api/eval') return json(await load('eval.json'));
  if (p === '/api/eval/label') {
    if (!(body.grade >= 0 && body.grade <= 3)) return json({ error: 'grade must be 0–3' }, 400);
    s.labels.push(body); return json({ ok: true });
  }
  if (p === '/api/skills') return json(await load('skills.json'));
  if (p === '/api/backends' || p === '/api/backends/refresh') return json(await backendsNow(s));
  if ((m = p.match(/^\/api\/backends\/([^/]+)\/key$/))) {
    const id = decodeURIComponent(m[1]);
    if (method === 'PUT') { if (!String(body.key || '').trim()) return json({ error: 'empty key' }, 400); s.keys.add(id); return json({ ok: true }); }
    if (method === 'DELETE') { s.keys.delete(id); return json({ ok: true }); }
  }
  if (p === '/api/backends/chatgpt') return json(chatgptInfo(s));
  if (p === '/api/backends/chatgpt/sign-in' && method === 'POST') {
    s.chatgptPending = Date.now();
    setTimeout(() => { s.chatgpt = true; s.chatgptPending = 0; }, 3000);
    return json({ authorize_url: 'about:blank#mock-sign-in', button_label: 'Continue with ChatGPT' });
  }
  if (p === '/api/backends/chatgpt/sign-out' && method === 'POST') { s.chatgpt = false; return json({ ok: true }); }
  if (p === '/api/sources' && method === 'GET') return json({ sources: s.sources, connectors: (await load('connectors.json')).connectors });
  if (p === '/api/sources' && method === 'POST') {
    const man = (await load('connectors.json')).connectors.find((x) => x.id === body.connector);
    if (!man) return json({ error: `unknown connector "${body.connector}"` }, 400);
    for (const k of man.config_schema?.required || []) if (body.config?.[k] == null || body.config[k] === '') return json({ error: `config: "${k}" is a required property` }, 400);
    const src = { id: ulid(), connector: body.connector, name: body.name || man.name, config: body.config || {}, default_origin: body.default_origin || man.default_origin, default_privacy: body.default_privacy || man.default_privacy, cursor: null, created_at: iso(Date.now() / 1000) };
    s.sources.push(src);
    return json(src);
  }
  if ((m = p.match(/^\/api\/sources\/([^/]+)(\/sync)?$/))) {
    const src = s.sources.find((x) => x.id === decodeURIComponent(m[1]));
    if (!src) return json({ error: 'no such source' }, 404);
    if (m[2] && method === 'POST') { await sleep(500); return json({ created: 3, updated: 1, unchanged: 212, duplicates: 0, deleted: 0, renamed: 0, pages: 1 }); }
    if (method === 'PUT') { Object.assign(src, Object.fromEntries(Object.entries(body).filter(([k]) => ['name', 'config', 'default_origin', 'default_privacy'].includes(k)))); return json(src); }
    if (method === 'DELETE') { s.sources = s.sources.filter((x) => x !== src); return json({ ok: true, items_deleted: 214 }); }
  }
  if (p === '/api/settings') {
    if (method === 'PUT') { for (const [k, v] of Object.entries(body)) { if (v === null) delete s.settings[k]; else s.settings[k] = v; } }
    return json(s.settings);
  }
  if (p === '/api/egress') return json({ entries: s.egress.slice(0, +(q.get('limit') || 100)) });
  if (p === '/api/mcp/snippets') return json(await load('snippets.json'));
  if (p === '/api/demo') {
    if (method === 'POST') s.demo = body.enabled !== false;
    return json({ active: s.demo, available: true, bundle: 'demo/brain.jsonl', items: c.ids.length });
  }
  if (p === '/api/status') return json({ version: '0.1.0', demo: s.demo, items: c.ids.length, events: 0, head: null, embedder: { id: 'mock', backend: 'hash', status: 'ready', fallback: false, local: true } });
  if (p === '/api/chat' && method === 'POST') {
    s.chats++;
    const hits = search(s, body.message || '', 5);
    const bk = (await backendsNow(s)).backends.find((b) => b.id === (body.backend || 'ollama'));
    const remote = bk && bk.is_local === false;
    if (remote) {
      const approved = new Set(body.approved || []);
      const ask = [...new Set(hits.map((x) => x.item_id))].filter((id) => privacyOf(s, s.index.get(id)) === 'ask' && !approved.has(id));
      if (ask.length) return sse([['consent_required', { ids: ask, chat_id: body.chat_id || (s.lastChat = ulid()) }, 120]]);
    }
    const seenT = new Set();
    const cited = hits.filter((h) => !seenT.has(h.title) && seenT.add(h.title)).slice(0, 4).map((h) => h.item_id);
    const backend = body.backend || 'ollama:qwen3-8b';
    const titles = [...new Set(hits.map((h) => h.title))].slice(0, 3).map((t) => `"${t}"`);
    const answer = hits.length
      ? `you have written about this before. the clearest traces are ${titles.join(', ')}. read together, they lean the same way: you keep returning to the question rather than settling it. the dots that carry it are lit in the brain.`
      : 'i could not find anything you wrote that touches this yet. that is an answer of a kind: this is new ground.';
    const words = answer.split(/(?<= )/);
    const evs = [];
    words.forEach((w, k) => {
      evs.push(['token', { text: w }, 28]);
      if (k === 8 && cited.length) evs.push(['cited', { ids: cited }, 0]);
    });
    evs.push(['usage', { backend, input: 812 + (body.message || '').length, output: words.length * 1.3 | 0, estimated: backend.startsWith('ollama') }, 10]);
    evs.push(['done', { chat_id: body.chat_id || ulid(), item_id: ulid() }, 10]);
    if (remote) s.egress.unshift({ seq: 1000 + s.chats, at: iso(Date.now() / 1000), actor: { model: bk.id }, kind: { type: 'egress', provider: bk.id, model: 'mock', purpose: 'chat', items: cited, chars: 900 + (body.message || '').length }, prev_hash: '', hash: '' });
    return sse(evs);
  }
  return json({ error: 'mock: no route for ' + method + ' ' + p }, 404);
}

// /api/backends with the session's keys and sign-ins applied.
async function backendsNow(s) {
  const b = structuredClone(await load('backends.json'));
  for (const x of b.backends) {
    if (x.id === 'chatgpt') { x.available = s.chatgpt; x.status = s.chatgpt ? 'ready' : 'not signed in'; }
    if (x.id === 'anthropic' || x.id === 'openrouter') { x.available = s.keys.has(x.id); x.status = x.available ? 'ready' : 'no api key'; }
    if (x.id === 'claude-cli') { x.available = !!s.settings.harness?.claude_cli?.enabled; x.status = x.available ? 'found, enabled at your request' : 'found, off'; }
  }
  if (s.settings.routing) b.routing = { ...b.routing, ...s.settings.routing };
  return b;
}
function chatgptInfo(s) {
  return {
    button_label: 'Continue with ChatGPT',
    notice: "uses your chatgpt plan through openai's sign-in for open-source apps (preview). you can cap this app's weekly usage in chatgpt settings → usage. signing out here does not disconnect anamnesis; remove it in chatgpt settings to revoke access.",
    usage_settings_url: 'https://chatgpt.com/#settings',
    status: { signed_in: s.chatgpt, plan_usage: null, email: s.chatgpt ? 'reader@example.org' : null, message: s.chatgpt ? null : 'not signed in' },
    sign_in: { in_progress: !!s.chatgptPending, last_error: null },
  };
}

export function installMock() {
  window.fetch = async (input, init = {}) => {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href);
    if (!url.pathname.includes('/api/')) return realFetch(input, init);
    const method = (init.method || 'GET').toUpperCase();
    let body = null;
    if (init.body) { try { body = JSON.parse(init.body); } catch { body = null; } }
    return handle(method, url, body || {});
  };
  window.__anamnesisMock = true;
}

export const mockFixtureUrl = (name) => new URL(name, BASE).href;
