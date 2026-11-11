// Config forms generated from connector manifests' config_schema.
import test from 'node:test';
import assert from 'node:assert/strict';
import { fieldsOf, collect } from '../lib/schemaform.js';

const folder = {
  type: 'object', required: ['path'],
  properties: {
    path: { type: 'string', minLength: 1 },
    extensions: { type: 'array', items: { type: 'string' } },
    include_hidden: { type: 'boolean' },
    kind: { enum: ['note', 'journal'] },
    max_file_bytes: { type: 'integer', minimum: 1 },
  },
};
const jsonApi = {
  type: 'object', required: ['url', 'fields'],
  properties: {
    url: { type: 'string', pattern: '^https?://' },
    fields: { type: 'object', required: ['id', 'body'], properties: { id: { type: 'string' }, body: { type: 'string' }, title: { type: 'string' } } },
    per_page: { type: 'integer', minimum: 1 },
  },
};

test('fields flatten nested objects and put required ones first', () => {
  const f = fieldsOf(jsonApi);
  assert.deepEqual(f.filter((x) => x.required).map((x) => x.path), ['url', 'fields.id', 'fields.body']);
  assert.deepEqual(fieldsOf(folder).map((x) => x.type), ['string', 'list', 'boolean', 'enum', 'integer']);
});

test('collect coerces and validates', () => {
  assert.deepEqual(collect(folder, { path: ' ~/notes ', extensions: 'md, txt\norg', include_hidden: true, kind: '', max_file_bytes: '2048' }),
    { config: { path: '~/notes', extensions: ['md', 'txt', 'org'], include_hidden: true, max_file_bytes: 2048 }, errors: {} });
  assert.deepEqual(collect(folder, { path: '', max_file_bytes: '0' }).errors, { path: 'needed', max_file_bytes: 'at least 1' });
  assert.deepEqual(collect(folder, { path: 'x', max_file_bytes: '1.5' }).errors, { max_file_bytes: 'a whole number' });
  const j = collect(jsonApi, { url: 'https://api.example/x', 'fields.id': 'id', 'fields.body': 'text', per_page: '' });
  assert.deepEqual(j, { config: { url: 'https://api.example/x', fields: { id: 'id', body: 'text' } }, errors: {} });
  assert.equal(collect(jsonApi, { url: 'ftp://x', 'fields.id': 'a', 'fields.body': 'b' }).errors.url, 'an http(s) url');
});
