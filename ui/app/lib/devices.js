// Sync across devices: the pure parts of the settings → devices tab (wording, the setup
// body, conflict versions), apart from the dom so node --test can check them.

// "just now", "4 min ago", "3 h ago", "2 days ago"; "never" for nothing.
export function ago(iso, now = Date.now()) {
  if (!iso) return 'never';
  const t = new Date(iso).getTime();
  if (isNaN(t)) return 'never';
  const s = Math.max(0, (now - t) / 1000);
  if (s < 45) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400 * 1.5) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}

// One line for a pass: "3 sent · 12 received · 1 from the inbox · 1 to settle".
export function reportLine(r) {
  if (!r) return '';
  const bits = [];
  if (r.exported) bits.push(`${r.exported} sent`);
  if (r.imported) bits.push(`${r.imported} received`);
  if (r.inbox) bits.push(`${r.inbox} from the inbox`);
  if (r.new_conflicts) bits.push(`${r.new_conflicts} to settle`);
  return bits.length ? bits.join(' · ') : 'nothing new';
}

// The body of POST /api/devices/setup from the form's values, or { error }.
export function setupBody(f) {
  const pass = (f.passphrase || '').trim() ? f.passphrase : '';
  if ([...pass].length < 8) return { error: 'the passphrase needs at least 8 characters' };
  const base = { passphrase: pass, ...(f.device_name && f.device_name.trim() ? { device_name: f.device_name.trim() } : {}) };
  if (f.kind === 's3') {
    for (const [k, what] of [['endpoint', 'the endpoint'], ['bucket', 'the bucket'], ['access_key', 'the access key'], ['secret_key', 'the secret key']]) {
      if (!(f[k] || '').trim()) return { error: `${what} is missing` };
    }
    if (!/^https:\/\//.test(f.endpoint.trim())) return { error: 'the endpoint must start with https://' };
    return { kind: 's3', endpoint: f.endpoint.trim(), bucket: f.bucket.trim(), region: (f.region || '').trim() || 'auto', prefix: (f.prefix || '').trim(), access_key: f.access_key.trim(), secret_key: f.secret_key.trim(), ...base };
  }
  if (!(f.path || '').trim()) return { error: 'choose a folder' };
  return { kind: 'folder', path: f.path.trim(), ...base };
}

// How a device stands, in a few words.
export function deviceState(d, now = Date.now()) {
  if (d.unlinked) return `unlinked by ${d.unlinked.by}`;
  if (d.error) return `cannot be read: ${d.error}`;
  return `${d.this ? 'this device · ' : ''}last sync ${ago(d.last_sync_at, now)}`;
}

// A conflict's versions, the shown one first, with what each says.
export function versionsOf(c) {
  return [...(c.versions || [])]
    .sort((a, b) => (b.shown - a.shown) || (b.hlc.t - a.hlc.t) || (b.hlc.c - a.hlc.c))
    .map((v) => ({
      version: v.version,
      from: v.device_name || v.device,
      shown: !!v.shown,
      title: v.item ? v.item.title : null,
      text: v.item ? v.item.body : null,
      deleted: !v.item,
    }));
}

// The title to show for a conflict: any version's title.
export function conflictTitle(c) {
  const v = (c.versions || []).find((x) => x.item);
  return v ? v.item.title : 'a deleted note';
}
