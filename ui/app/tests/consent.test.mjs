// The chat's consent flow for items marked "ask", and settings merges.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createConsent, setChat, withApprovals, onConsentRequired, approve, deny, resetConsent } from '../lib/consent.js';
import { deepMerge, mergedBody } from '../lib/settings.js';

test('approve resends the same body with the ids, and remembers them for the chat', () => {
  const c = createConsent();
  const body = withApprovals(c, { message: 'what did i decide?', backend: 'anthropic' });
  assert.deepEqual(body, { message: 'what did i decide?', backend: 'anthropic' });
  const r = onConsentRequired(c, body, { ids: ['A', 'B', 'A'], chat_id: 'C1' });
  assert.deepEqual(r, { kind: 'ask', ids: ['A', 'B'] });
  const next = approve(c);
  assert.deepEqual(next, { message: 'what did i decide?', backend: 'anthropic', chat_id: 'C1', approved: ['A', 'B'] });
  assert.equal(c.pending, null);
  // the next message in the same chat carries the approvals
  const b2 = withApprovals(c, { message: 'and then?', backend: 'anthropic' });
  assert.deepEqual(b2, { message: 'and then?', backend: 'anthropic', chat_id: 'C1', approved: ['A', 'B'] });
  // a further ask adds to them
  onConsentRequired(c, b2, { ids: ['D'], chat_id: 'C1' });
  assert.deepEqual(approve(c).approved, ['A', 'B', 'D']);
});

test('deny sends nothing to the same backend; it can ask a local one instead', () => {
  const c = createConsent();
  const body = { message: 'q', backend: 'anthropic' };
  onConsentRequired(c, body, { ids: ['A'], chat_id: 'C1' });
  assert.equal(deny(c), null);
  assert.equal(c.pending, null);
  onConsentRequired(c, body, { ids: ['A'], chat_id: 'C1' });
  assert.deepEqual(deny(c, 'ollama'), { message: 'q', backend: 'ollama', chat_id: 'C1' });
  assert.equal(c.approved.size, 0);
});

test('a repeated ask for ids already approved is a loop, not a new prompt', () => {
  const c = createConsent();
  const r = onConsentRequired(c, { message: 'q', approved: ['A'] }, { ids: ['A'], chat_id: 'C' });
  assert.equal(r.kind, 'loop');
  assert.equal(c.pending, null);
  assert.equal(approve(c), null);
});

test('approvals belong to one chat', () => {
  const c = createConsent();
  setChat(c, 'C1');
  c.approved.add('A');
  setChat(c, 'C1');
  assert.ok(c.approved.has('A'));
  setChat(c, 'C2');
  assert.equal(c.approved.size, 0);
  c.approved.add('B');
  resetConsent(c);
  assert.equal(c.chatId, null);
  assert.deepEqual(withApprovals(c, { message: 'x' }), { message: 'x' });
});

test('settings patches are merged into the whole top-level value', () => {
  const current = { harness: { claude_cli: { enabled: false, model: 'opus' }, openai_compatible: [{ id: 'a' }] }, routing: { chat: 'a', label: 'b' }, theme: { id: 'aphesis' } };
  assert.deepEqual(mergedBody(current, { harness: { claude_cli: { enabled: true } } }), { harness: { claude_cli: { enabled: true, model: 'opus' }, openai_compatible: [{ id: 'a' }] } });
  assert.deepEqual(mergedBody(current, { routing: { chat: null } }), { routing: { label: 'b' } });
  assert.deepEqual(mergedBody(current, { privacy: { default: 'ask' } }), { privacy: { default: 'ask' } });
  assert.deepEqual(mergedBody(current, { theme: null }), { theme: null });
  // arrays replace
  assert.deepEqual(deepMerge({ a: [1, 2] }, { a: [3] }), { a: [3] });
  assert.deepEqual(deepMerge(undefined, { a: { b: 1 } }), { a: { b: 1 } });
});
