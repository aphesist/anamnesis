import test from 'node:test';
import assert from 'node:assert/strict';
import { renderMarkdown } from '../lib/md.js';

test('escapes html before formatting', () => {
  const out = renderMarkdown('<script>alert(1)</script> **bold**');
  assert.ok(!out.includes('<script>'));
  assert.ok(out.includes('&lt;script&gt;'));
  assert.ok(out.includes('<strong>bold</strong>'));
});

test('headings, lists, quotes and wikilinks', () => {
  const out = renderMarkdown('# title\n\n- a\n- b\n\n> quoted\n\nsee [[other note]]');
  assert.ok(out.includes('<h2>title</h2>'));
  assert.ok(out.includes('<ul><li>a</li><li>b</li></ul>'));
  assert.ok(out.includes('<blockquote>quoted</blockquote>'));
  assert.ok(out.includes('data-title="other note"'));
});

test('only http links become anchors', () => {
  assert.ok(!renderMarkdown('[x](javascript:alert(1))').includes('<a href="javascript'));
  assert.ok(renderMarkdown('[x](https://example.org)').includes('href="https://example.org"'));
});

test('aliased wikilinks show the alias and keep the target', () => {
  const out = renderMarkdown('but Mr. [[Henry David Thoreau|Thoreau]] walked');
  assert.ok(out.includes('data-title="Henry David Thoreau">Thoreau</a>'));
  assert.ok(!out.includes('|'));
});
