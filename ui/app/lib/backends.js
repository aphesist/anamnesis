// Pure helpers over the rows of GET /api/backends (docs/contracts.md). No DOM, so
// they are unit-tested in node. A row is
//   { id, kind, label, available, status, capabilities, terms, notice?, is_local?, leaves_machine?, key_storage? }
// `kind` is what the harness serialises: openai_compatible, anthropic, gemini, chatgpt,
// claude_cli, or retrieval for the built-in responder. Older fixtures used local,
// api_key, chatgpt_signin and cli; those still map.

const KIND_ALIASES = { local: 'openai_compatible', api_key: 'openai_compatible', chatgpt_signin: 'chatgpt', cli: 'claude_cli', claude: 'claude_cli' };
export const KINDS = ['openai_compatible', 'anthropic', 'gemini', 'chatgpt', 'claude_cli', 'retrieval'];

export function normKind(kind) {
  const k = String(kind || '').toLowerCase().replace(/-/g, '_');
  return KIND_ALIASES[k] || (KINDS.includes(k) ? k : 'other');
}

// True when a url's host is loopback (the only hosts the harness treats as local by default).
export function isLoopbackUrl(u) {
  if (!u) return false;
  let host;
  try { host = new URL(u).hostname; } catch { return false; }
  host = host.replace(/^\[|\]$/g, '').toLowerCase();
  return host === 'localhost' || host === '::1' || /^127\./.test(host) || host.endsWith('.localhost');
}

// The harness entry for an openai-compatible backend in settings.harness, if any.
export function harnessEntry(b, settings) {
  const list = settings?.harness?.openai_compatible;
  return Array.isArray(list) ? list.find((c) => c && c.id === b.id) : undefined;
}

// Does a call to this backend stay on this machine? Uses the server's flags when present,
// then the configured base url / `local` override, then the cost model.
export function isLocal(b, settings) {
  if (!b) return false;
  if (typeof b.is_local === 'boolean') return b.is_local;
  if (typeof b.leaves_machine === 'boolean') return !b.leaves_machine;
  const k = normKind(b.kind);
  if (k === 'retrieval') return true;
  if (k === 'openai_compatible') {
    const e = harnessEntry(b, settings);
    if (e && typeof e.local === 'boolean') return e.local;
    const url = b.base_url || e?.base_url;
    if (url) return isLoopbackUrl(url);
    if (String(b.kind).toLowerCase() === 'local') return true; // legacy fixtures
    return b.capabilities?.cost_model === 'free_local';
  }
  return false; // anthropic, gemini, chatgpt, claude_cli all call out
}
export const leavesMachine = (b, settings) => {
  if (b && typeof b.leaves_machine === 'boolean') return b.leaves_machine;
  return !isLocal(b, settings);
};

// "stays on this machine" / "leaves this machine, logged"
export const egressLabel = (b, settings) => (isLocal(b, settings) ? 'stays on this machine' : 'leaves this machine, logged');

// Display label: lowercase, except the exact "Continue with ChatGPT" wording, which
// is a product label the sign-in terms ask to keep as is.
export function displayLabel(b) {
  const label = b?.label || b?.id || '';
  if (normKind(b?.kind) === 'chatgpt' && /chatgpt/i.test(label) && /continue/i.test(label)) return label;
  return label.toLowerCase();
}
export const keepsCase = (b) => displayLabel(b) !== String(b?.label || b?.id || '').toLowerCase();

// Which kinds take an api key through PUT /api/backends/:id/key.
export function takesKey(b, settings) {
  const k = normKind(b?.kind);
  if (k === 'anthropic' || k === 'gemini') return true;
  if (k === 'openai_compatible') return !isLocal(b, settings);
  return false;
}

// How a backend signs in, for the connections panel.
export function access(b) {
  switch (normKind(b?.kind)) {
    case 'chatgpt': return 'sign-in';
    case 'claude_cli': return 'cli';
    case 'retrieval': return 'none';
    case 'anthropic': case 'gemini': return 'key';
    default: return 'endpoint';
  }
}

export const termsGrey = (b) => b?.terms === 'grey' || b?.capabilities?.terms === 'grey';

// Where saved api keys live, for the "keys are kept in …" line. The server may send it
// per backend or once at the top level of the response.
export function keyStorage(b, resp) {
  const v = b?.key_storage ?? resp?.key_storage;
  if (!v) return null;
  if (typeof v === 'string') return v.replace(/_/g, ' ');
  if (typeof v === 'object') return String(v.label || v.kind || v.backend || '').replace(/_/g, ' ') || null;
  return null;
}

// The initial choice for the chat picker: the routed chat backend when it is usable,
// else the first available one with clear terms, else retrieval.
export function initialChoice(resp) {
  const list = resp?.backends || [];
  const routed = list.find((b) => b.id === resp?.routing?.chat);
  if (routed && routed.available !== false && !termsGrey(routed)) return routed.id;
  return (list.find((b) => b.available && !termsGrey(b) && normKind(b.kind) !== 'retrieval') || list.find((b) => normKind(b.kind) === 'retrieval') || {}).id || null;
}
