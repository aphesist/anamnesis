// Settings → devices: wording, the setup body and conflict versions.
import test from 'node:test';
import assert from 'node:assert/strict';
import { ago, reportLine, setupBody, deviceState, versionsOf, conflictTitle } from '../lib/devices.js';

const NOW = Date.parse('2026-11-12T12:00:00Z');

test('times read as ago', () => {
  assert.equal(ago(null, NOW), 'never');
  assert.equal(ago('2026-11-12T11:59:40Z', NOW), 'just now');
  assert.equal(ago('2026-11-12T11:56:00Z', NOW), '4 min ago');
  assert.equal(ago('2026-11-12T09:00:00Z', NOW), '3 h ago');
  assert.equal(ago('2026-11-09T12:00:00Z', NOW), '3 days ago');
});

test('a pass in one line', () => {
  assert.equal(reportLine({ exported: 3, imported: 12, inbox: 1, new_conflicts: 1 }), '3 sent · 12 received · 1 from the inbox · 1 to settle');
  assert.equal(reportLine({ exported: 0, imported: 0, inbox: 0, new_conflicts: 0 }), 'nothing new');
  assert.equal(reportLine(null), '');
});

test('the setup body checks what it needs', () => {
  assert.deepEqual(setupBody({ kind: 'folder', path: ' ~/Dropbox/anamnesis ', passphrase: 'walden at dawn' }),
    { kind: 'folder', path: '~/Dropbox/anamnesis', passphrase: 'walden at dawn' });
  assert.equal(setupBody({ kind: 'folder', path: '', passphrase: 'walden at dawn' }).error, 'choose a folder');
  assert.equal(setupBody({ kind: 'folder', path: '/x', passphrase: 'short' }).error, 'the passphrase needs at least 8 characters');
  const s3 = setupBody({ kind: 's3', endpoint: 'https://acct.r2.cloudflarestorage.com', bucket: 'notes', access_key: 'a', secret_key: 'b', passphrase: 'walden at dawn', device_name: 'phone' });
  assert.deepEqual(s3, { kind: 's3', endpoint: 'https://acct.r2.cloudflarestorage.com', bucket: 'notes', region: 'auto', prefix: '', access_key: 'a', secret_key: 'b', passphrase: 'walden at dawn', device_name: 'phone' });
  assert.equal(setupBody({ kind: 's3', endpoint: 'http://x', bucket: 'b', access_key: 'a', secret_key: 'b', passphrase: 'walden at dawn' }).error, 'the endpoint must start with https://');
  assert.equal(setupBody({ kind: 's3', endpoint: 'https://x', passphrase: 'walden at dawn' }).error, 'the bucket is missing');
});

test('devices and conflicts in words', () => {
  assert.equal(deviceState({ this: true, last_sync_at: '2026-11-12T11:56:00Z' }, NOW), 'this device · last sync 4 min ago');
  assert.equal(deviceState({ unlinked: { by: 'laptop' } }, NOW), 'unlinked by laptop');
  assert.equal(deviceState({ error: 'modified' }, NOW), 'cannot be read: modified');
  const c = { versions: [
    { version: 'a', device_name: 'laptop', shown: false, hlc: { t: 1, c: 0 }, item: { title: 'plan', body: 'laptop words' } },
    { version: 'b', device_name: 'phone', shown: true, hlc: { t: 2, c: 0 }, item: null },
  ] };
  const vs = versionsOf(c);
  assert.deepEqual(vs.map((v) => [v.version, v.from, v.shown, v.deleted]), [['b', 'phone', true, true], ['a', 'laptop', false, false]]);
  assert.equal(conflictTitle(c), 'plan');
  assert.equal(conflictTitle({ versions: [{ item: null }] }), 'a deleted note');
});
