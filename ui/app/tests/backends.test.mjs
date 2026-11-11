// The backend-kind mapping: the server's kinds (openai_compatible, anthropic, gemini,
// chatgpt, claude_cli, retrieval) and where a call to each goes.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { normKind, isLocal, leavesMachine, egressLabel, displayLabel, takesKey, access, termsGrey, keyStorage, initialChoice, isLoopbackUrl } from '../lib/backends.js';

const fixture = JSON.parse(readFileSync(new URL('../mock/backends.json', import.meta.url), 'utf8'));

test('server kinds map to themselves; legacy fixture kinds still map', () => {
  for (const k of ['openai_compatible', 'anthropic', 'gemini', 'chatgpt', 'claude_cli', 'retrieval']) assert.equal(normKind(k), k);
  assert.equal(normKind('chatgpt_signin'), 'chatgpt');
  assert.equal(normKind('local'), 'openai_compatible');
  assert.equal(normKind('cli'), 'claude_cli');
  assert.equal(normKind('claude-cli'), 'claude_cli');
  assert.equal(normKind('something_new'), 'other');
  assert.equal(normKind(undefined), 'other');
});

test('the flags from the server win', () => {
  assert.equal(isLocal({ kind: 'anthropic', is_local: true }), true);
  assert.equal(isLocal({ kind: 'openai_compatible', leaves_machine: true }), false);
  assert.equal(leavesMachine({ kind: 'retrieval', leaves_machine: true }), true);
});

test('without flags: retrieval stays, hosted kinds leave, openai-compatible goes by its host', () => {
  assert.equal(isLocal({ kind: 'retrieval' }), true);
  for (const k of ['anthropic', 'gemini', 'chatgpt', 'claude_cli']) assert.equal(isLocal({ kind: k }), false, k);
  const settings = { harness: { openai_compatible: [
    { id: 'ollama', base_url: 'http://127.0.0.1:11434/v1' },
    { id: 'lan', base_url: 'http://192.168.1.4:8080/v1' },
    { id: 'forced', base_url: 'https://my.box/v1', local: true },
    { id: 'or', base_url: 'https://openrouter.ai/api/v1' },
  ] } };
  assert.equal(isLocal({ id: 'ollama', kind: 'openai_compatible' }, settings), true);
  assert.equal(isLocal({ id: 'lan', kind: 'openai_compatible' }, settings), false);
  assert.equal(isLocal({ id: 'forced', kind: 'openai_compatible' }, settings), true);
  assert.equal(isLocal({ id: 'or', kind: 'openai_compatible' }, settings), false);
  // not in settings (a probed local server): the cost model decides
  assert.equal(isLocal({ id: 'llamacpp', kind: 'openai_compatible', capabilities: { cost_model: 'free_local' } }), true);
  assert.equal(isLocal({ id: 'x', kind: 'openai_compatible', capabilities: { cost_model: 'metered' } }), false);
  assert.equal(egressLabel({ kind: 'retrieval' }), 'stays on this machine');
  assert.equal(egressLabel({ kind: 'anthropic' }), 'leaves this machine, logged');
});

test('loopback detection', () => {
  for (const u of ['http://127.0.0.1:11434/v1', 'http://localhost:8080', 'http://[::1]:1234/v1', 'http://127.1.2.3']) assert.ok(isLoopbackUrl(u), u);
  for (const u of ['https://openrouter.ai/api/v1', 'http://10.0.0.2', 'not a url', '', 'http://localhost.evil.com']) assert.ok(!isLoopbackUrl(u), u);
});

test('labels are lowercase, except the exact chatgpt sign-in label', () => {
  assert.equal(displayLabel({ kind: 'anthropic', label: 'Anthropic (API key)' }), 'anthropic (api key)');
  assert.equal(displayLabel({ kind: 'chatgpt', label: 'Continue with ChatGPT' }), 'Continue with ChatGPT');
  assert.equal(displayLabel({ kind: 'chatgpt', label: 'chatgpt plan' }), 'chatgpt plan');
});

test('keys, access and terms', () => {
  assert.ok(takesKey({ kind: 'anthropic' }));
  assert.ok(takesKey({ kind: 'gemini' }));
  assert.ok(!takesKey({ kind: 'chatgpt' }));
  assert.ok(!takesKey({ kind: 'claude_cli' }));
  assert.ok(!takesKey({ kind: 'openai_compatible', is_local: true }));
  assert.ok(takesKey({ kind: 'openai_compatible', is_local: false }));
  assert.equal(access({ kind: 'chatgpt' }), 'sign-in');
  assert.equal(access({ kind: 'claude_cli' }), 'cli');
  assert.ok(termsGrey({ terms: 'grey' }));
  assert.ok(termsGrey({ capabilities: { terms: 'grey' } }));
  assert.equal(keyStorage({ key_storage: 'os_keychain' }), 'os keychain');
  assert.equal(keyStorage({}, { key_storage: 'file' }), 'file');
  assert.equal(keyStorage({}, {}), null);
});

test('the initial chat choice skips grey and unavailable backends', () => {
  assert.equal(initialChoice({ backends: [{ id: 'cli', kind: 'claude_cli', available: true, terms: 'grey' }, { id: 'retrieval', kind: 'retrieval', available: true }], routing: { chat: 'cli' } }), 'retrieval');
  assert.equal(initialChoice({ backends: [{ id: 'a', kind: 'anthropic', available: true, terms: 'allowed' }, { id: 'retrieval', kind: 'retrieval', available: true }], routing: { chat: 'retrieval' } }), 'retrieval');
  assert.equal(initialChoice({ backends: [{ id: 'a', kind: 'anthropic', available: true, terms: 'allowed' }, { id: 'retrieval', kind: 'retrieval', available: true }], routing: {} }), 'a');
});

test('the backends fixture has the server shape', () => {
  const kinds = new Set(fixture.backends.map((b) => b.kind));
  for (const k of kinds) assert.notEqual(normKind(k), 'other', k);
  assert.ok(kinds.has('retrieval'));
  for (const b of fixture.backends) {
    for (const k of ['id', 'kind', 'label', 'available', 'status', 'capabilities', 'terms']) assert.ok(k in b, `${b.id} ${k}`);
    assert.ok(['allowed', 'grey'].includes(b.terms));
  }
  for (const t of ['chat', 'label', 'extract', 'summarise']) assert.ok(t in fixture.routing, t);
  assert.equal(typeof fixture.detected, 'boolean');
});
