// Distil: the pure parts of the chat panel's "distil" flow (labels, the order focus
// moves in, the summary line), kept apart from the dom so node --test can check them.

export const KIND_WORDS = {
  idea: 'idea',
  decision: 'decision',
  open_question: 'open question',
  plan: 'plan',
};

export const kindLabel = (k) => KIND_WORDS[k] || String(k || '').replace(/_/g, ' ');

// Is the extractor a version of the rule-based one ("rule", "rule:v2")?
export const isRule = (extractor) => extractor === 'rule' || String(extractor || '').startsWith('rule:');

// Who wrote the proposal's title, in the panel's words.
export function titleBy(p) {
  return isRule(p.extractor) ? 'title from your words' : 'title written by a model';
}

// The distiller, in plain words: "rules", "a local model (qwen)", "example (m1)".
export function distillerWords(d) {
  if (!d || isRule(d)) return 'the rules (no model)';
  const [who, ...rest] = String(d).split(':');
  const model = rest.join(':');
  if (who === 'local') return `a local model${model ? ` (${model})` : ''}`;
  return `${who}${model ? ` (${model})` : ''}`;
}

// One line under the proposals: how many, by what, and why the model was not used.
export function summary(outcome) {
  const n = (outcome.proposals || []).length;
  const what = n === 0 ? 'nothing to keep in your words this time' : n === 1 ? 'one note in your words' : `${n} notes in your words`;
  const by = `by ${distillerWords(outcome.distiller)}`;
  return [what, by, outcome.note || null].filter(Boolean).join(' · ');
}

// After a card is answered, where focus goes: the next unanswered card after `i`, else
// the previous one, else -1 (all answered).
export function nextOpen(answered, i) {
  for (let j = i + 1; j < answered.length; j++) if (!answered[j]) return j;
  for (let j = i - 1; j >= 0; j--) if (!answered[j]) return j;
  return -1;
}

// Keys on a focused card: k / enter keep, d / delete / backspace drop, arrows move.
export function keyAction(key) {
  switch (key) {
    case 'k': case 'K': return 'keep';
    case 'd': case 'D': case 'Delete': case 'Backspace': return 'drop';
    case 'ArrowDown': case 'j': return 'next';
    case 'ArrowUp': return 'prev';
    default: return null;
  }
}
