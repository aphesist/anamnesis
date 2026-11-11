// Dev-only generator for the mock fixtures in this folder.
// Run with: node ui/app/mock/gen-fixtures.mjs
// Output is deterministic (seeded) so regenerating does not churn the diff.
// The fixtures follow the http api shapes in docs/contracts.md exactly.

import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const N = 4800;

// ---------- seeded randomness ----------
let seed = 0x5eed1e55;
function rnd() {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const pick = (a) => a[Math.floor(rnd() * a.length)];
const gauss = () => { let u = 0, v = 0; while (!u) u = rnd(); while (!v) v = rnd(); return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v); };
const CROCK = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
function ulid(ms) {
  let s = '';
  let t = ms;
  for (let i = 0; i < 10; i++) { s = CROCK[t % 32] + s; t = Math.floor(t / 32); }
  for (let i = 0; i < 16; i++) s += CROCK[Math.floor(rnd() * 32)];
  return s;
}
const iso = (sec) => new Date(sec * 1000).toISOString().replace(/\.\d{3}Z$/, 'Z');

// ---------- time ----------
const T0 = Date.UTC(2024, 0, 3) / 1000;
const T1 = Date.UTC(2026, 10, 10) / 1000;
const DAY = 86400;

// ---------- regions ----------
// dir: rough direction in the semantic space, chosen so the anatomy warp lands
// each region near its ornamental anatomical name. born: fraction of the span.
const REGIONS = [
  { key: 'attention', label: 'ethics of attention', anatomy: 'prefrontal cortex', dir: [0.1, 0.6, 1.0], born: 0.0, w: 1.3 },
  { key: 'walking', label: 'walking & weather', anatomy: 'cerebellum', dir: [0.2, -0.8, -1.0], born: 0.0, w: 1.0 },
  { key: 'emerson', label: "emerson's essays", anatomy: 'angular gyrus', dir: [-1.0, 0.4, -0.3], born: 0.38, w: 1.0, parent: 'reading' },
  { key: 'thoreau', label: "thoreau's journals", anatomy: 'supramarginal gyrus', dir: [-0.9, 0.7, 0.2], born: 0.38, w: 0.9, parent: 'reading' },
  { key: 'type', label: 'the craft of type', anatomy: 'fusiform gyrus', dir: [0.9, -0.5, -0.2], born: 0.05, w: 0.8 },
  { key: 'money', label: 'money, rent, plans', anatomy: 'orbitofrontal cortex', dir: [0.4, -0.4, 1.0], born: 0.02, w: 0.7 },
  { key: 'sleep', label: 'sleep and dreams', anatomy: 'hippocampus', dir: [-0.3, -0.3, 0.1], born: 0.1, w: 0.7 },
  { key: 'code', label: 'code & tools', anatomy: 'motor cortex', dir: [0.5, 1.0, 0.0], born: 0.0, w: 1.2 },
  { key: 'friends', label: 'friends, letters', anatomy: 'temporal pole', dir: [1.0, -0.3, 0.7], born: 0.15, w: 0.8 },
  { key: 'music', label: 'music heard', anatomy: 'superior temporal gyrus', dir: [-1.0, -0.4, 0.5], born: 0.6, w: 0.8, mergedFrom: ['songs', 'concerts'] },
  { key: 'grief', label: 'grief, slowly', anatomy: 'cingulate gyrus', dir: [0.0, 0.9, -0.4], born: 0.83, w: 0.5 },
  { key: 'garden', label: 'garden log', anatomy: 'visual cortex', dir: [0.1, 0.2, -1.1], born: 0.82, w: 0.6 },
];

const VOCAB = {
  attention: {
    a: ['attention', 'distraction', 'the feed', 'silence', 'noticing', 'boredom', 'a phone-free day', 'slowness', 'care', 'deep work', 'looking'],
    t: ['on %a', '%a as a moral act', 'what %a asks of me', 'notes on %a', 'against %a', '%a, again', 'a rule for %a', 'why %a matters', 'weil on %a'],
  },
  walking: {
    a: ['the canal', 'the ridge', 'low fog', 'first frost', 'rain', 'the old quarry', 'a long loop', 'wind from the north', 'the river path', 'dusk'],
    t: ['walk: %a', '%a, 6 km', 'out early, %a', 'weather log: %a', '%a and a heron', 'cold hands, %a', 'walked to %a'],
  },
  emerson: {
    a: ['self-reliance', 'the over-soul', 'circles', 'experience', 'nature', 'compensation', 'the poet', 'history', 'friendship'],
    t: ['emerson, "%a"', 'reading "%a"', 'quote from "%a"', 'margin note: %a', '"%a" and my week', 'what "%a" gets wrong'],
  },
  thoreau: {
    a: ['walden', 'the journal, 1851', 'walking', 'economy', 'where i lived', 'solitude', 'sounds', 'the bean-field', 'winter animals'],
    t: ['thoreau on %a', 'reading %a', 'a line from %a', '%a: notes', 'excerpt: %a', 'thoreau, %a, and rent'],
  },
  type: {
    a: ['kerning', 'a typewriter face', 'stipple', 'brush strokes', 'the baseline', 'ligatures', 'lowercase', 'a variable axis', 'specimen pages'],
    t: ['on %a', 'sketch: %a', '%a study', 'why %a', 'fixing %a', 'notes on %a', '%a, close up'],
  },
  money: {
    a: ['rent', 'the budget', 'a freelance invoice', 'savings', 'moving out', 'the lease', 'taxes', 'a cheaper flat', 'subscriptions'],
    t: ['plan: %a', 'decided: %a', '%a for next year', 'numbers on %a', 'cancel %a?', 'what %a costs', 'question: %a'],
  },
  sleep: {
    a: ['a dream about water', 'insomnia', 'waking at four', 'a recurring house', 'falling', 'lucid', 'the old school', 'naps'],
    t: ['dream: %a', '%a', 'slept badly, %a', 'night note: %a', '%a again'],
  },
  code: {
    a: ['sqlite', 'the event log', 'webgl points', 'a rust crate', 'umap', 'embeddings', 'a shell alias', 'git rebase', 'hash chains', 'the parser'],
    t: ['%a notes', 'how %a works', 'bug in %a', 'rejected: %a', 'trying %a', 'til: %a', '%a, a sketch', 'chat: about %a'],
  },
  friends: {
    a: ['m.', 'a letter to j.', 'dinner with a. and k.', 'r.\'s wedding', 'an old friend', 'a postcard', 'the call with s.'],
    t: ['%a', 'letter: %a', 'after %a', 'note to self after %a', 'reply to %a'],
  },
  music: {
    a: ['a string quartet', 'satie at night', 'a choir in the rain', 'a record from 1972', 'the drone piece', 'a small gig', 'bach, slowly'],
    t: ['heard: %a', '%a', 'listening to %a', 'concert: %a', 'on %a'],
  },
  grief: {
    a: ['the hospital', 'her garden', 'the funeral', 'old photographs', 'a voicemail', 'the first spring after', 'the empty chair'],
    t: ['%a', 'about %a', 'writing it down: %a', 'a month after: %a'],
  },
  garden: {
    a: ['tomatoes', 'the compost', 'first beans', 'slugs', 'sweet peas', 'frost cloth', 'seed list', 'the quince'],
    t: ['garden: %a', '%a, planted', '%a log', 'harvest: %a', 'plan: %a'],
  },
  unsorted: {
    a: ['a clip', 'a link', 'a scan', 'an untitled note', 'a screenshot', 'a fragment'],
    t: ['%a', 'untitled: %a', 'inbox: %a'],
  },
};

const KINDS = ['note', 'journal', 'quote', 'article', 'pdf', 'book', 'chat', 'clip', 'feed', 'skill'];
const KIND_MIX = {
  attention: { note: 5, journal: 2, quote: 3, article: 2, chat: 2 },
  walking: { journal: 8, note: 2 },
  emerson: { quote: 5, book: 2, note: 3, pdf: 1 },
  thoreau: { quote: 5, book: 2, note: 3, pdf: 1 },
  type: { note: 4, clip: 3, article: 2, pdf: 1 },
  money: { note: 5, journal: 1, chat: 1 },
  sleep: { journal: 7, note: 1 },
  code: { note: 4, chat: 4, article: 2, clip: 1, feed: 1, skill: 0.4 },
  friends: { journal: 3, note: 3 },
  music: { journal: 3, note: 2, clip: 1, feed: 1 },
  grief: { journal: 6, note: 2 },
  garden: { journal: 5, note: 2 },
  unsorted: { clip: 3, feed: 2, note: 1, pdf: 1 },
};
function pickW(obj) {
  const e = Object.entries(obj); const s = e.reduce((a, [, w]) => a + w, 0);
  let r = rnd() * s; for (const [k, w] of e) { r -= w; if (r <= 0) return k; } return e[0][0];
}
function speakerFor(kind) {
  if (kind === 'chat') return rnd() < 0.55 ? 1 : 0;
  if (['article', 'pdf', 'book', 'clip', 'feed', 'quote'].includes(kind)) return rnd() < 0.85 ? 2 : 0;
  if (kind === 'skill') return 0;
  return rnd() < 0.96 ? 0 : 1;
}

// ---------- points ----------
const norm = (v) => { const l = Math.hypot(...v); return v.map((x) => x / l); };
const regionCenters = REGIONS.map((r) => norm(r.dir).map((x) => x * 1.25));
const wsum = REGIONS.reduce((a, r) => a + r.w, 0);

const pts = [];
for (let n = 0; n < N; n++) {
  const unsorted = rnd() < 0.04;
  let ri = -1;
  if (!unsorted) {
    let r = rnd() * wsum;
    for (ri = 0; ri < REGIONS.length; ri++) { r -= REGIONS[ri].w; if (r <= 0) break; }
    ri = Math.min(ri, REGIONS.length - 1);
  }
  const R = ri >= 0 ? REGIONS[ri] : null;
  const key = R ? R.key : 'unsorted';
  // time: after the region's birth, denser lately
  const born = R ? R.born : 0.1;
  let f = born + (1 - born) * Math.pow(rnd(), 0.8);
  if (R && R.parent && rnd() < 0.45) f = 0.05 + rnd() * 0.33; // items written before the split
  if (R && R.mergedFrom && rnd() < 0.5) f = 0.2 + rnd() * 0.4;
  const t = Math.round(T0 + f * (T1 - T0) + rnd() * DAY);
  // position: a few sub-blobs per region
  let p;
  if (R) {
    const c = regionCenters[ri];
    const sub = Math.floor(rnd() * 3);
    const off = [Math.sin(sub * 2.1 + ri) * 0.22, Math.cos(sub * 1.7 + ri) * 0.18, Math.sin(sub * 3.3 - ri) * 0.2];
    const s = 0.16 + 0.06 * rnd();
    p = [c[0] + off[0] + gauss() * s, c[1] + off[1] + gauss() * s, c[2] + off[2] + gauss() * s];
  } else {
    p = [gauss() * 0.7, gauss() * 0.6, gauss() * 0.8];
  }
  const kind = pickW(KIND_MIX[key]);
  const v = VOCAB[key];
  const title = pick(v.t).replace('%a', pick(v.a));
  pts.push({ ri, key, t, p, kind, speaker: speakerFor(kind), title });
}
pts.sort((a, b) => a.t - b.t);
pts.forEach((q) => { q.id = ulid(q.t * 1000); });

// ---------- regions with ids ----------
const regionIds = REGIONS.map((r) => ulid((T0 + r.born * (T1 - T0)) * 1000));
const regions = REGIONS.map((r, i) => {
  const mine = pts.filter((q) => q.ri === i);
  const c = [0, 0, 0];
  mine.forEach((q) => { c[0] += q.p[0]; c[1] += q.p[1]; c[2] += q.p[2]; });
  return {
    id: regionIds[i], label: r.label, anatomy: r.anatomy, size: mine.length,
    centroid: c.map((x) => +(x / Math.max(1, mine.length)).toFixed(3)),
    born_at: iso(Math.round(T0 + r.born * (T1 - T0))),
  };
});

// ---------- links ----------
const byRegion = new Map();
pts.forEach((q, i) => { if (!byRegion.has(q.ri)) byRegion.set(q.ri, []); byRegion.get(q.ri).push(i); });
const links = [];
const seen = new Set();
const LINK_KINDS = ['written', 'written', 'written', 'manual', 'accepted_suggestion'];
for (let k = 0; k < 2000; k++) {
  const i = Math.floor(rnd() * N);
  let j;
  if (rnd() < 0.82) { const same = byRegion.get(pts[i].ri); j = pick(same); } else j = Math.floor(rnd() * N);
  if (i === j) continue;
  const a = Math.min(i, j), b = Math.max(i, j);
  const key = a + ':' + b; if (seen.has(key)) continue; seen.add(key);
  links.push([b, a, pick(LINK_KINDS)]); // from the later item to the earlier one
}

const cloud = {
  ids: pts.map((q) => q.id),
  xyz: pts.flatMap((q) => q.p.map((x) => +x.toFixed(3))),
  region: pts.map((q) => q.ri),
  kind: pts.map((q) => KINDS.indexOf(q.kind)),
  speaker: pts.map((q) => q.speaker),
  t: pts.map((q) => q.t),
  titles: pts.map((q) => q.title),
  kinds: KINDS,
  regions,
  links,
};

// ---------- timeline ----------
const months = [];
for (let y = 2024; y <= 2026; y++) for (let m = 1; m <= 12; m++) {
  const ym = `${y}-${String(m).padStart(2, '0')}`;
  if (ym >= '2024-01' && ym <= '2026-11') months.push(ym);
}
const monthOf = (sec) => iso(sec).slice(0, 7);
const dayMap = new Map();
pts.forEach((q) => { const d = iso(q.t).slice(0, 10); dayMap.set(d, (dayMap.get(d) || 0) + 1); });
const days = [...dayMap.entries()].sort().map(([date, count]) => ({ date, count }));
const by_region = regions.map((r, i) => ({ region_id: r.id, counts: months.map((m) => pts.filter((q) => q.ri === i && monthOf(q.t) === m).length) }));
const spk = (s) => months.map((m) => pts.filter((q) => q.speaker === s && monthOf(q.t) === m).length);

// ---------- threads ----------
const TH = {
  decision: [
    ['move to the cheaper flat in spring', 'money'], ['keep the phone out of the bedroom', 'attention'],
    ['write every morning before email', 'attention'], ['use sqlite, not a server database', 'code'],
    ['walk the ridge loop on sundays', 'walking'], ['grow fewer things, better', 'garden'],
    ['answer letters by hand', 'friends'], ['one typeface for the whole site', 'type'],
  ],
  open_question: [
    ['is learning recollection?', 'emerson'], ['what do i owe the people i quote?', 'thoreau'],
    ['why do i dream of the old school?', 'sleep'], ['does the feed shape what i notice?', 'attention'],
    ['should the event log be the only truth?', 'code'], ['how do i keep her garden?', 'grief'],
    ['what would a calmer week look like?', 'attention'], ['can stipple carry meaning, not just mood?', 'type'],
    ['is rent the real constraint?', 'money'],
  ],
  plan: [
    ['read walden again in winter', 'thoreau'], ['build the brain view with webgl points', 'code'],
    ['save three months of rent', 'money'], ['plant beans after the last frost', 'garden'],
    ['visit m. in march', 'friends'], ['record one concert a month', 'music'],
    ['a type specimen for the journal', 'type'],
  ],
  rejection: [
    ['rejected: a cloud-only notes app', 'code'], ['rejected: buying a car', 'money'],
    ['rejected: the dark-pattern newsletter', 'attention'], ['rejected: umap in the browser', 'code'],
    ['rejected: a second phone', 'attention'],
  ],
};
const threads = [];
for (const [kind, list] of Object.entries(TH)) {
  for (const [statement, key] of list) {
    const ri = REGIONS.findIndex((r) => r.key === key);
    const cands = byRegion.get(ri) || [0];
    const idx = pick(cands);
    const q = pts[idx];
    const status = kind === 'open_question' ? (rnd() < 0.75 ? 'open' : 'resolved') : kind === 'plan' ? pick(['open', 'open', 'resolved', 'abandoned']) : kind === 'decision' ? pick(['open', 'resolved', 'superseded']) : 'resolved';
    threads.push({
      id: ulid(q.t * 1000), kind, status, statement, item_id: q.id,
      quote: statement.replace(/^rejected: /, 'i will not do this: ') + (/[.?!]$/.test(statement) ? '' : '.'),
      resolved_by: status === 'resolved' || status === 'superseded' ? pick(pts.filter((z) => z.t > q.t).map((z) => z.id)) || null : null,
      created_at: iso(q.t), updated_at: iso(Math.min(T1, q.t + 40 * DAY)),
      extractor: rnd() < 0.5 ? 'rule' : 'local:qwen3-4b',
    });
  }
}
const thmonth = (k) => months.map((m) => threads.filter((th) => th.kind === k && th.created_at.slice(0, 7) === m).length);
// pad the lifecycle series with the implied smaller counts so the chart has texture
const padded = (k, scale) => thmonth(k).map((v, i) => v + Math.round(Math.max(0, Math.sin(i * 0.7 + scale) * 2 + rnd() * 2)));

const timeline = {
  days, months, by_region,
  by_speaker: { you: spk(0), assistants: spk(1), other: spk(2) },
  threads_by_month: {
    decision: padded('decision', 0), open_question: padded('open_question', 1.3),
    plan: padded('plan', 2.1), rejection: padded('rejection', 3.2).map((v) => Math.max(0, v - 1)),
  },
};

// ---------- items (a few with full bodies; mock.js synthesises the rest) ----------
const BODIES = {
  attention: 'attention is the rarest and purest form of generosity, weil says. i keep testing that against the week: what did i actually look at, and what looked at me?\n\n- the feed took forty minutes before nine.\n- the heron took ten, and gave them back.\n\ndecided: the phone sleeps in the hall.',
  walking: 'out before seven. low fog on the canal, the towpath soft. a heron stood so still i doubted it.\n\n6 km. cold hands, warm head. came back with the question about rent half-answered.',
  emerson: '> "to believe your own thought, to believe that what is true for you in your private heart is true for all men, — that is genius."\n\nwhat if the links i make by hand were already there, waiting? is learning recollection?',
  code: 'the event log is the only truth: every change is appended, hash-chained, and replayable. the brain is a view, not a database.\n\nrejected: umap in the browser — too slow for 50k points, and the layout should be stable anyway.',
};

const skills = [
  { id: ulid(T1 * 1000 - 9e9), title: 'weekly review', body: '# weekly review\n\n1. list what i wrote this week, by area.\n2. name the decisions made and the questions still open.\n3. pick one thing to drop.\n\ncite every claim.' },
  { id: ulid(T1 * 1000 - 8e9), title: 'what did i reject', body: '# what did i reject\n\nfind every rejection in the last six months. for each: what i chose instead, and whether i still agree.' },
  { id: ulid(T1 * 1000 - 7e9), title: 'questions still open', body: '# questions still open\n\nlist open questions older than a month, oldest first, with the note that raised each.' },
  { id: ulid(T1 * 1000 - 6e9), title: 'a letter from the archive', body: '# a letter from the archive\n\nchoose one note from a year ago today and write me a short letter about what has changed since.' },
  { id: ulid(T1 * 1000 - 5e9), title: 'latent links', body: '# latent links\n\nfor the note in front of me, find three older notes it already agrees with, and one it quietly contradicts.' },
];

// ---------- review ----------
const REASONS = ['due: you marked this to return in a month', 'contextual: near what you wrote today', 'serendipitous: from a quiet corner of the brain', 'due: open question, unanswered for 90 days', 'contextual: shares a thread with "rent"', 'serendipitous: written on this day in 2024'];
const reviewIdx = [12, 340, 811, 1500, 2203, 2950].map((i) => Math.min(i, N - 1));

// ---------- lineage ----------
const runAt = (f) => iso(Math.round(T0 + f * (T1 - T0)));
const reg = (i, size) => ({ id: regionIds[i], label: REGIONS[i].label, anatomy: REGIONS[i].anatomy, size, born_at: regions[i].born_at, died_at: null, parent_ids: [] });
const readingId = ulid((T0 + 0.01 * (T1 - T0)) * 1000);
const songsId = ulid((T0 + 0.2 * (T1 - T0)) * 1000);
const concertsId = ulid((T0 + 0.25 * (T1 - T0)) * 1000);
const mk = (id, label, anatomy, size, bornF, diedF) => ({ id, label, anatomy, size, born_at: runAt(bornF), died_at: diedF == null ? null : runAt(diedF), parent_ids: [] });
const idx = (k) => REGIONS.findIndex((r) => r.key === k);
const lineage = {
  runs: [
    {
      run_id: ulid((T0 + 0.02 * (T1 - T0)) * 1000), at: runAt(0.02),
      regions: [reg(idx('attention'), 60), reg(idx('walking'), 40), reg(idx('code'), 55), reg(idx('money'), 20), mk(readingId, 'reading notes', 'angular gyrus', 70, 0.01, null)],
      transitions: [{ type: 'birth', region: regionIds[idx('attention')] }, { type: 'birth', region: regionIds[idx('walking')] }, { type: 'birth', region: regionIds[idx('code')] }, { type: 'birth', region: regionIds[idx('money')] }, { type: 'birth', region: readingId }],
    },
    {
      run_id: ulid((T0 + 0.2 * (T1 - T0)) * 1000), at: runAt(0.2),
      regions: [reg(idx('type'), 80), reg(idx('sleep'), 70), reg(idx('friends'), 60), mk(songsId, 'songs', 'superior temporal gyrus', 30, 0.2, null)],
      transitions: [{ type: 'birth', region: regionIds[idx('type')] }, { type: 'birth', region: regionIds[idx('sleep')] }, { type: 'birth', region: regionIds[idx('friends')] }, { type: 'birth', region: songsId }, { type: 'survive', region: readingId }],
    },
    {
      run_id: ulid((T0 + 0.38 * (T1 - T0)) * 1000), at: runAt(0.38),
      regions: [{ ...reg(idx('emerson'), 160), parent_ids: [readingId] }, { ...reg(idx('thoreau'), 140), parent_ids: [readingId] }, mk(readingId, 'reading notes', 'angular gyrus', 300, 0.01, 0.38), mk(concertsId, 'concerts', 'heschl\'s gyrus', 25, 0.38, null)],
      transitions: [{ type: 'split', from: readingId, into: [regionIds[idx('emerson')], regionIds[idx('thoreau')]] }, { type: 'birth', region: concertsId }],
    },
    {
      run_id: ulid((T0 + 0.6 * (T1 - T0)) * 1000), at: runAt(0.6),
      regions: [{ ...reg(idx('music'), 110), parent_ids: [songsId, concertsId] }, mk(songsId, 'songs', 'superior temporal gyrus', 60, 0.2, 0.6), mk(concertsId, 'concerts', 'heschl\'s gyrus', 50, 0.38, 0.6)],
      transitions: [{ type: 'merge', from: [songsId, concertsId], into: regionIds[idx('music')] }],
    },
    {
      run_id: ulid((T0 + 0.83 * (T1 - T0)) * 1000), at: runAt(0.83),
      regions: [reg(idx('grief'), 40), reg(idx('garden'), 50)],
      transitions: [{ type: 'birth', region: regionIds[idx('grief')] }, { type: 'birth', region: regionIds[idx('garden')] }, { type: 'survive', region: regionIds[idx('attention')] }],
    },
  ],
};

// ---------- eval ----------
const evalFix = {
  links: {
    title: 'latent links', question: 'what share of the links i later made by hand was already latent in what i had written?',
    n: 214, estimate: 0.41, interval: [0.34, 0.48], status: 'enough data', basis: 'measured',
    unit: 'share', detail: 'hand links that matched a top-10 suggestion made before them. control arm (held out, never shown): 0.37 [0.27, 0.48], n 52.',
  },
  retrieval: {
    title: 'retrieval', question: 'does search find what i meant?',
    n: 63, estimate: 0.72, interval: [0.64, 0.79], status: 'enough data', basis: 'measured',
    unit: 'ndcg@10', detail: 'from your own relevance labels (grade 0–3).',
  },
  resurfacing: {
    title: 'resurfacing', question: 'is the algorithm better than a random pick?',
    n: 38, estimate: 0.11, interval: [-0.08, 0.30], status: 'measuring', basis: 'measured',
    unit: 'difference in useful-rate', detail: 'micro-randomised: algorithm 0.58 vs random 0.47. too early to say.',
  },
  growth: {
    title: 'growth', question: 'is the brain still finding new regions?',
    n: 5, estimate: 0.8, interval: [0.4, 1.4], status: 'measuring', basis: 'modelled',
    unit: 'new regions per quarter', detail: 'modelled from 5 clustering runs; the slowing is a hypothesis, not yet a finding.',
  },
};

// ---------- backends ----------
const backends = {
  backends: [
    { id: 'ollama:qwen3-8b', kind: 'local', label: 'ollama · qwen3 8b', available: true, capabilities: ['chat', 'label', 'extract'], status: 'running on this machine', terms: 'allowed' },
    { id: 'anthropic:api', kind: 'api_key', label: 'anthropic api key', available: true, capabilities: ['chat', 'label', 'extract'], status: 'key saved', terms: 'allowed' },
    { id: 'openai:chatgpt', kind: 'chatgpt_signin', label: 'Continue with ChatGPT', available: false, capabilities: ['chat'], status: 'not signed in', terms: 'allowed' },
    { id: 'claude-cli', kind: 'cli', label: 'claude cli (installed)', available: true, capabilities: ['chat'], status: 'found on path', terms: 'grey', notice: 'runs your own installed claude cli under your own subscription. its terms for third-party harnesses are unclear; anamnesis only calls it at your request and never stores its credentials.' },
    { id: 'openrouter', kind: 'api_key', label: 'openrouter', available: false, capabilities: ['chat'], status: 'no key', terms: 'allowed' },
  ],
  routing: { chat: 'ollama:qwen3-8b', label: 'ollama:qwen3-8b', extract: 'ollama:qwen3-8b' },
};

const extra = {
  bodies: BODIES,
  review_idx: reviewIdx,
  reasons: REASONS,
  region_keys: REGIONS.map((r) => r.key),
};

const w = (name, obj) => writeFileSync(join(here, name), JSON.stringify(obj) + '\n');
w('cloud.json', cloud);
w('timeline.json', timeline);
w('threads.json', { threads });
w('lineage.json', lineage);
w('eval.json', evalFix);
w('backends.json', backends);
w('skills.json', { skills });
w('extra.json', extra);
console.log('points', N, 'links', links.length, 'regions', regions.length, 'threads', threads.length);
