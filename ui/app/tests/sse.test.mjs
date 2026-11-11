import test from 'node:test';
import assert from 'node:assert/strict';
import { parseSSE } from '../lib/sse.js';

test('parses complete events and keeps the unfinished tail', () => {
  const { events, rest } = parseSSE('event: token\ndata: {"text":"hel"}\n\nevent: token\ndata: {"text":"lo"}\n\nevent: cit');
  assert.deepEqual(events, [['token', { text: 'hel' }], ['token', { text: 'lo' }]]);
  assert.equal(rest, 'event: cit');
});

test('handles crlf, multi-line data, default event name and non-json data', () => {
  const { events } = parseSSE('data: {"a":\r\ndata: 1}\r\n\r\nevent: error\ndata: plain words\n\n');
  assert.deepEqual(events, [['message', { a: 1 }], ['error', { text: 'plain words' }]]);
});

test('streams split at any byte still yield the same events', () => {
  const full = 'event: cited\ndata: {"ids":["A","B"]}\n\nevent: done\ndata: {"chat_id":"C","item_id":"D"}\n\n';
  for (let cut = 1; cut < full.length; cut += 7) {
    let buf = '', got = [];
    for (const part of [full.slice(0, cut), full.slice(cut)]) { const r = parseSSE(buf + part); got = got.concat(r.events); buf = r.rest; }
    assert.equal(got.length, 2);
    assert.deepEqual(got[0], ['cited', { ids: ['A', 'B'] }]);
  }
});
