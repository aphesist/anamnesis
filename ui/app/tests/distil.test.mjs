// The chat panel's distil flow: labels, summary line, focus order and keys.
import test from 'node:test';
import assert from 'node:assert/strict';
import { kindLabel, titleBy, distillerWords, summary, nextOpen, keyAction, isRule } from '../lib/distil.js';

test('kinds read as words', () => {
  assert.equal(kindLabel('open_question'), 'open question');
  assert.equal(kindLabel('idea'), 'idea');
  assert.equal(kindLabel('something_new'), 'something new');
});

test('a model-written title is said to be one', () => {
  assert.equal(titleBy({ extractor: 'rule:v2' }), 'title from your words');
  assert.equal(titleBy({ extractor: 'local:qwen3' }), 'title written by a model');
  assert.equal(titleBy({ extractor: 'anthropic:claude-haiku-5-5' }), 'title written by a model');
  assert.ok(isRule('rule') && isRule('rule:v3') && !isRule('ruler:x'));
});

test('the distiller and the summary line', () => {
  assert.equal(distillerWords('rule:v2'), 'the rules (no model)');
  assert.equal(distillerWords('local:qwen3:8b'), 'a local model (qwen3:8b)');
  assert.equal(distillerWords('example:m1'), 'example (m1)');
  assert.equal(summary({ proposals: [], distiller: 'rule:v2' }), 'nothing to keep in your words this time · by the rules (no model)');
  assert.equal(
    summary({ proposals: [{}, {}], distiller: 'rule:v2', note: 'the model could not be reached: the rules ran' }),
    '2 notes in your words · by the rules (no model) · the model could not be reached: the rules ran');
  assert.equal(summary({ proposals: [{}], distiller: 'local:q' }), 'one note in your words · by a local model (q)');
});

test('focus moves to the next open card, then back, then nowhere', () => {
  assert.equal(nextOpen([true, false, false], 0), 1);
  assert.equal(nextOpen([false, false, true], 2), 1);
  assert.equal(nextOpen([false, true, true], 2), 0);
  assert.equal(nextOpen([true, true], 1), -1);
});

test('keys on a card', () => {
  assert.equal(keyAction('k'), 'keep');
  assert.equal(keyAction('d'), 'drop');
  assert.equal(keyAction('Delete'), 'drop');
  assert.equal(keyAction('ArrowDown'), 'next');
  assert.equal(keyAction('ArrowUp'), 'prev');
  assert.equal(keyAction('x'), null);
});
