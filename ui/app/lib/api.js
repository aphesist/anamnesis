// Thin client for the http api in docs/contracts.md.
// Same-origin by default (the server serves ui/); override with ?api=http://host:port.

const params = new URLSearchParams(location.search);
export const API_BASE = (params.get('api') || '').replace(/\/$/, '');

async function req(method, path, body) {
  const init = { method, headers: {} };
  if (body !== undefined) {
    init.headers['content-type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  const res = await fetch(API_BASE + path, init);
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status}`);
  const ct = res.headers.get('content-type') || '';
  return ct.includes('json') ? res.json() : res.text();
}

export const api = {
  get: (p) => req('GET', p),
  post: (p, b) => req('POST', p, b ?? {}),
  put: (p, b) => req('PUT', p, b ?? {}),
  del: (p) => req('DELETE', p),
  cloud: () => req('GET', '/api/cloud'),
  item: (id) => req('GET', '/api/items/' + encodeURIComponent(id)),
  createItem: (b) => req('POST', '/api/items', b),
  updateItem: (id, b) => req('PUT', '/api/items/' + encodeURIComponent(id), b),
  search: (q, k = 30) => req('GET', `/api/search?q=${encodeURIComponent(q)}&k=${k}`),
  timeline: () => req('GET', '/api/timeline'),
  threads: (kind, status) => req('GET', '/api/threads?' + new URLSearchParams({ ...(kind ? { kind } : {}), ...(status ? { status } : {}) })),
  setThread: (id, status, resolved_by) => req('POST', '/api/threads/' + encodeURIComponent(id), { status, ...(resolved_by ? { resolved_by } : {}) }),
  review: () => req('GET', '/api/review'),
  rate: (rid, useful) => req('POST', '/api/review/' + encodeURIComponent(rid), { useful }),
  suggestions: (item) => req('GET', '/api/suggestions?item=' + encodeURIComponent(item)),
  decide: (sid, accept) => req('POST', '/api/suggestions/' + encodeURIComponent(sid), { accept }),
  lineage: () => req('GET', '/api/lineage'),
  eval: () => req('GET', '/api/eval'),
  skills: () => req('GET', '/api/skills'),
  backends: () => req('GET', '/api/backends'),
};

// POST /api/chat and parse the text/event-stream response.
// onEvent(name, data) is called per event; resolves when the stream ends.
export async function streamChat(body, onEvent, signal) {
  const res = await fetch(API_BASE + '/api/chat', {
    method: 'POST', headers: { 'content-type': 'application/json', accept: 'text/event-stream' },
    body: JSON.stringify(body), signal,
  });
  if (!res.ok || !res.body) throw new Error('chat: ' + res.status);
  const reader = res.body.getReader();
  const dec = new TextDecoder();
  let buf = '';
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += dec.decode(value, { stream: true });
    let cut;
    while ((cut = buf.search(/\r?\n\r?\n/)) >= 0) {
      const block = buf.slice(0, cut);
      buf = buf.slice(cut).replace(/^\r?\n\r?\n/, '');
      let ev = 'message'; const data = [];
      for (const line of block.split(/\r?\n/)) {
        if (line.startsWith('event:')) ev = line.slice(6).trim();
        else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
      }
      if (!data.length) continue;
      let parsed; try { parsed = JSON.parse(data.join('\n')); } catch { parsed = { text: data.join('\n') }; }
      onEvent(ev, parsed);
    }
  }
}
