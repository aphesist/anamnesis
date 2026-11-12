// Settings → devices: sync this brain with the user's other devices through storage they
// manage (a folder their cloud drive syncs, or their own s3-compatible bucket), end-to-end
// encrypted. Linked devices, last sync, unlink, conflicts to settle, and the plain inbox.
import { h, clear, fmtDate } from '../lib/dom.js';
import { api } from '../lib/api.js';
import { ago, reportLine, setupBody, deviceState, versionsOf, conflictTitle } from '../lib/devices.js';

const sect = (title, ...kids) => h('section.cx-sect', h('h3.cx-h', title), ...kids);
const meta = (...t) => h('p.meta', ...t);
const errText = (e) => (e && (e.serverMessage || e.message)) || 'something went wrong';
const excerpt = (s, n = 280) => (s && s.length > n ? s.slice(0, n - 1) + '…' : s || '');

export async function mountDevices(el, ctx) {
  el.classList.add('cx', 'devices');
  let st;
  try { st = await api.get('/api/devices'); } catch (e) { el.append(meta('the server could not be reached: ' + errText(e))); return; }
  const reload = async () => { clear(el); await mountDevices(el, ctx); };
  el.append(meta('keep this brain on your other devices through storage you already have: a folder your own cloud drive syncs (onedrive, dropbox, google drive, icloud, syncthing) or your own s3-compatible bucket. everything is encrypted on this device with your passphrase; the storage only ever holds ciphertext. there is no anamnesis account and nothing goes to us.'));
  if (st.demo_active) el.append(h('p.notice', 'the demo brain is open. sync works on your own brain, never on the demo.'));
  if (!st.configured) { el.append(setupForm(st, reload)); return; }

  // ---- where, and now
  const p = st.place || {};
  const msg = h('span.meta', { 'aria-live': 'polite' });
  const now = h('button.textbtn.strong', { type: 'button', onclick: async () => {
    now.disabled = true; msg.textContent = 'syncing…';
    try { const v = await api.post('/api/devices/sync'); msg.textContent = reportLine(v.report) || 'done.'; setTimeout(reload, 900); }
    catch (e) { msg.textContent = errText(e); now.disabled = false; }
  } }, 'sync now');
  el.append(sect('sync',
    h('div.cx-routing',
      h('span.meta', 'where'), h('span.keep-case', p.location || ''),
      h('span.meta', 'last sync'), h('span', st.last_sync_at ? `${ago(st.last_sync_at)} · ${fmtDate(st.last_sync_at, true)}` : 'not yet'),
      h('span.meta', 'last pass'), h('span', st.last_report ? reportLine(st.last_report) : '—')),
    st.last_error ? h('p.notice', 'the last pass failed: ', h('span.keep-case', st.last_error)) : null,
    h('div.btnrow', now, msg),
    meta(`a pass runs every ${st.interval_minutes || 5} minutes while the app is open.`)));

  // ---- the inbox
  el.append(sect('inbox',
    meta('anything that drops a markdown or text file here (a phone notes app, a share-sheet shortcut, another computer) is taken in on the next sync as your own writing, then moved to inbox/imported/. the inbox is not encrypted: it is a plain folder you write to.'),
    h('pre.keep-case', { tabindex: '0', 'aria-label': 'the inbox' }, p.inbox || '')));

  // ---- conflicts
  const conflicts = st.conflicts || [];
  if (conflicts.length) {
    const box = sect(conflicts.length === 1 ? 'one note to settle' : `${conflicts.length} notes to settle`,
      meta('these were changed on two devices before either saw the other. both versions are kept; the newer one is shown until you choose.'));
    for (const c of conflicts) box.append(conflictBlock(c, reload, ctx));
    el.append(box);
  }

  // ---- devices
  const list = h('ul.list.cx-devices');
  for (const d of st.devices || []) {
    const row = h('li.cx-device',
      h('div.cx-line', h('span.keep-case', d.name), d.platform ? h('span.meta', ` · ${d.platform}`) : null),
      h('div.meta', deviceState(d)));
    if (!d.this && !d.unlinked) {
      const sure = h('span.btnrow', { hidden: true },
        h('span.meta', 'its later changes will be ignored, and it stops when it next syncs. it still knows the passphrase: to lock a lost device out, start a new vault with a new passphrase.'),
        h('button.textbtn.strong', { type: 'button', onclick: async () => {
          try { await api.post(`/api/devices/${encodeURIComponent(d.id)}/unlink`); reload(); } catch (e) { ctx.toast?.(errText(e)); }
        } }, 'unlink it'),
        h('button.textbtn', { type: 'button', onclick: () => { sure.hidden = true; ask.hidden = false; } }, 'keep it'));
      const ask = h('button.textbtn', { type: 'button', onclick: () => { ask.hidden = true; sure.hidden = false; } }, 'unlink…');
      row.append(h('div.btnrow', ask, sure));
    }
    list.append(row);
  }
  el.append(sect('devices', list));

  // ---- stop
  const stopSure = h('span.btnrow', { hidden: true },
    h('span.meta', 'this device forgets the place and the key. your brain here and the sync folder stay as they are.'),
    h('button.textbtn.strong', { type: 'button', onclick: async () => { try { await api.del('/api/devices'); reload(); } catch (e) { ctx.toast?.(errText(e)); } } }, 'stop'),
    h('button.textbtn', { type: 'button', onclick: () => { stopSure.hidden = true; stopAsk.hidden = false; } }, 'keep syncing'));
  const stopAsk = h('button.textbtn', { type: 'button', onclick: () => { stopAsk.hidden = true; stopSure.hidden = false; } }, 'stop syncing on this device…');
  el.append(h('div.btnrow', stopAsk, stopSure));
}

function conflictBlock(c, reload, ctx) {
  const vs = versionsOf(c);
  const block = h('div.cx-conflict', { role: 'group', 'aria-label': `two versions of ${conflictTitle(c)}` },
    h('div.cx-line', h('button.textbtn.keep-case', { type: 'button', onclick: () => ctx.open?.(c.item_id) }, conflictTitle(c))));
  const choose = async (body) => {
    block.querySelectorAll('button').forEach((b) => { b.disabled = true; });
    try { await api.post(`/api/devices/conflicts/${encodeURIComponent(c.id)}`, body); reload(); }
    catch (e) { ctx.toast?.(errText(e)); block.querySelectorAll('button').forEach((b) => { b.disabled = false; }); }
  };
  for (const v of vs) {
    block.append(h('div.cx-version',
      h('div.meta', `from ${v.from}${v.shown ? ' · shown now' : ''}`),
      v.deleted ? h('p.soft', 'deleted on that device') : h('blockquote.keep-case', excerpt(v.text)),
      h('button.textbtn.strong', { type: 'button', onclick: () => choose({ keep: 'one', version: v.version }) }, v.deleted ? 'delete it' : 'keep this one')));
  }
  if (vs.filter((v) => !v.deleted).length > 1) {
    block.append(h('div.btnrow', h('button.textbtn', { type: 'button', onclick: () => choose({ keep: 'all' }) }, 'keep all, the others as separate notes')));
  }
  return block;
}

function setupForm(st, reload) {
  let kind = 'folder';
  const f = (name, attrs = {}) => h('input.field', { name, autocomplete: 'off', spellcheck: 'false', ...attrs });
  const path = f('path', { placeholder: '~/Dropbox/anamnesis', 'aria-label': 'sync folder' });
  const s3 = {
    endpoint: f('endpoint', { placeholder: 'https://<account>.r2.cloudflarestorage.com', 'aria-label': 'endpoint' }),
    bucket: f('bucket', { placeholder: 'bucket', 'aria-label': 'bucket' }),
    region: f('region', { placeholder: 'region (auto for r2)', 'aria-label': 'region' }),
    prefix: f('prefix', { placeholder: 'folder in the bucket (optional)', 'aria-label': 'folder in the bucket' }),
    access_key: f('access_key', { placeholder: 'access key id', 'aria-label': 'access key id' }),
    secret_key: f('secret_key', { type: 'password', placeholder: 'secret access key', 'aria-label': 'secret access key' }),
  };
  const pass = f('passphrase', { type: 'password', placeholder: 'passphrase', 'aria-label': 'passphrase' });
  const name = f('device_name', { placeholder: 'this device’s name (optional)', 'aria-label': 'device name' });
  const folderBox = h('div.cx-addform', path, h('p.meta', 'a folder your cloud drive or syncthing already syncs. on the other devices, choose the same folder as they see it.'));
  const s3Box = h('div.cx-addform', { hidden: true }, s3.endpoint, s3.bucket, s3.region, s3.prefix, s3.access_key, s3.secret_key,
    h('p.meta', 'a bucket you created yourself; the keys go to this device’s keychain, never into settings. give them access to this bucket only.'));
  const pick = h('div.choice', { role: 'radiogroup', 'aria-label': 'where' });
  for (const [k, label] of [['folder', 'a folder'], ['s3', 'an s3 bucket']]) {
    const b = h('button', { type: 'button', role: 'radio', 'aria-checked': String(k === kind), onclick: () => {
      kind = k;
      for (const x of pick.children) x.setAttribute('aria-checked', String(x === b));
      folderBox.hidden = k !== 'folder'; s3Box.hidden = k !== 's3';
    } }, label);
    pick.append(b);
  }
  const msg = h('p.meta', { 'aria-live': 'polite' });
  const go = h('button.textbtn.strong', { type: 'submit' }, 'start syncing');
  const form = h('form.cx-addform', pick, folderBox, s3Box, name, pass,
    h('p.meta', 'the same passphrase on every device. it never leaves this device and cannot be recovered: without it the folder is unreadable, also to you. the first device makes the vault; the others join it.'),
    h('div.btnrow', go), msg);
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const values = { kind, path: path.value, passphrase: pass.value, device_name: name.value };
    for (const [k, inp] of Object.entries(s3)) values[k] = inp.value;
    const body = setupBody(values);
    if (body.error) { msg.textContent = body.error; return; }
    go.disabled = true; msg.textContent = 'deriving the key from the passphrase (a second or two)…';
    try {
      const v = await api.post('/api/devices/setup', body);
      pass.value = ''; s3.secret_key.value = '';
      msg.textContent = v.created ? 'a new vault was made there. this is its first device.' : 'joined the vault there.';
      setTimeout(reload, 1200);
    } catch (err) { msg.textContent = errText(err); go.disabled = false; }
  });
  return sect('set up sync', form);
}
