// Settings writes. PUT /api/settings replaces each top-level key it is sent (the server
// may move to a deep merge, but the ui does not rely on that): so a change to one field
// of `harness` or `routing` is merged into the current value and the whole key is sent.

const isObj = (v) => v != null && typeof v === 'object' && !Array.isArray(v);

// Deep merge: objects merge key by key, arrays and scalars replace, null deletes.
export function deepMerge(base, patch) {
  if (!isObj(patch)) return patch === undefined ? base : patch;
  const out = isObj(base) ? { ...base } : {};
  for (const [k, v] of Object.entries(patch)) {
    if (v === null) delete out[k];
    else if (isObj(v)) out[k] = deepMerge(out[k], v);
    else if (v !== undefined) out[k] = v;
  }
  return out;
}

// The body to PUT for a patch over the current settings: only the touched top-level keys,
// each carrying its whole merged value (a top-level null still removes the key).
export function mergedBody(current, patch) {
  const body = {};
  for (const [k, v] of Object.entries(patch || {})) {
    body[k] = v === null ? null : isObj(v) ? deepMerge(current?.[k], v) : v;
  }
  return body;
}

// GET, merge, PUT. Returns all settings as the server answers them.
export async function patchSettings(api, patch) {
  let current = {};
  try { current = (await api.get('/api/settings')) || {}; } catch { current = {}; }
  return api.put('/api/settings', mergedBody(current, patch));
}
