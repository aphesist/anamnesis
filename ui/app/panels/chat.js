// Chat: streams tokens from POST /api/chat (server-sent events), lights the
// cited dots in the brain, lets the user pick a backend (with its terms status,
// notice and whether it leaves the machine) from /api/backends, asks before any
// item marked "ask" leaves the machine (consent_required), and takes one-tap
// relevance grades on the citations. "distil" proposes a few notes quoting the user's own
// messages in this chat; each is kept or dropped with one tap (or k / d on a focused card).
import { h, clear } from '../lib/dom.js';
import { store, on, BIT, setBit, clearBit } from '../lib/store.js';
import { api, streamChat } from '../lib/api.js';
import { normKind, isLocal, egressLabel, displayLabel, keepsCase, termsGrey, initialChoice } from '../lib/backends.js';
import { createConsent, setChat, resetConsent, withApprovals, onConsentRequired, approve, deny } from '../lib/consent.js';
import { gradeControl } from '../lib/grade.js';
import { kindLabel, titleBy, summary as distilSummary, nextOpen, keyAction } from '../lib/distil.js';

let unread = 0;

// Once an answer has finished streaming: [[target|alias]] wikilinks read as their alias, and
// [n] markers become small links to the n-th cited note (the order the server cited them in).
function tidy(textEl, citedIds) {
  const parts = [...textEl.childNodes];
  if (parts.some((n) => n.nodeType !== 3)) return; // an error or note was appended: leave it
  const text = textEl.textContent
    .replace(/\[\[([^\]|]+)\|([^\]]+)\]\]/g, '$2')
    .replace(/\[\[([^\]]+)\]\]/g, '$1');
  clear(textEl);
  for (const bit of text.split(/(\[\d{1,2}\])/)) {
    const m = bit.match(/^\[(\d{1,2})\]$/);
    const id = m ? citedIds[+m[1] - 1] : null;
    if (id) textEl.append(h('button.cite-mark', { type: 'button', title: 'open the cited note', onclick: () => window.anamnesis?.ctx.open(id) }, m[1]));
    else textEl.append(bit);
  }
}
export default {
  id: 'chat',
  title: 'chat',
  badge: () => unread || '',
  mount(el, ctx) {
    el.classList.add('chat');
    let resp = null, backends = [], settings = null, chosen = null, busy = null, accepted = new Set(), noticeFor = null, chatId = null;
    const consent = createConsent();
    const picker = h('div.backend-picker', { role: 'radiogroup', 'aria-label': 'model' });
    const pickerWrap = h('details.sect', h('summary', 'model'), picker);
    const chosenLabel = h('span.meta.chosen', '');
    const log = h('div.chat-log', { 'aria-live': 'polite', 'aria-relevant': 'additions' });
    const input = h('textarea', { placeholder: 'ask your notes something…', 'aria-label': 'message', rows: 3 });
    const send = h('button.textbtn.strong', { type: 'submit' }, 'send');
    const stop = h('button.textbtn', { type: 'button', hidden: true, onclick: () => busy?.abort() }, 'stop');
    const newChat = () => { resetConsent(consent); chatId = null; distilBtn.disabled = true; clear(log).append(emptyNote()); clearBit(BIT.CITED); };
    const fresh = h('button.textbtn', { type: 'button', title: 'start a new chat', onclick: newChat }, 'new chat');
    const distilBtn = h('button.textbtn', { type: 'button', disabled: true, title: 'propose a few notes from your own words in this chat', onclick: () => distil() }, 'distil');
    const form = h('form.chat-form', chosenLabel, input, h('div.btnrow', send, stop, distilBtn, fresh));
    const emptyNote = () => h('p.empty', 'ask anything. answers cite your own notes, and the cited dots light up in the brain.');
    log.append(emptyNote());
    // a fresh install opens on the demo brain: say where chats go, and offer the way back
    const demoNote = h('p.notice.demo-note', { role: 'status', hidden: true });
    const drawDemoNote = () => {
      clear(demoNote).append(h('span', "you're in the demo brain — chats are stored there. "),
        h('button.textbtn.strong', { type: 'button', onclick: () => ctx.setDemo?.(false) }, 'switch to your brain'));
      demoNote.hidden = !store.demo;
    };
    drawDemoNote();
    on('demo', drawDemoNote);
    on('brain-switched', () => { drawDemoNote(); newChat(); });
    el.append(demoNote, pickerWrap, log, form);

    const byId = (id) => backends.find((b) => b.id === id);
    const nameOf = (b) => h(keepsCase(b) ? 'span.brand-label' : 'span', displayLabel(b));
    const localBackend = () => {
      const avail = backends.filter((b) => b.available !== false && isLocal(b, settings));
      return avail.find((b) => normKind(b.kind) !== 'retrieval') || avail.find((b) => normKind(b.kind) === 'retrieval') || null;
    };

    const renderPicker = () => {
      clear(picker);
      for (const b of backends) {
        const isOn = chosen === b.id;
        const grey = termsGrey(b);
        const local = isLocal(b, settings);
        const btn = h('button.be', { type: 'button', role: 'radio', 'aria-checked': String(isOn), disabled: b.available === false, title: b.status || '' },
          h('span', { 'aria-hidden': 'true' }, isOn ? '●' : '○'),
          h('span', nameOf(b), h('span.meta', ` · ${b.status || (b.available === false ? 'unavailable' : 'ready')}`),
            h(`span.where.${local ? 'here' : 'away'}`, local ? 'stays here' : 'leaves, logged')),
          h(`span.terms.${grey ? 'grey' : 'ok'}`, grey ? 'terms: grey' : 'terms: allowed'));
        btn.addEventListener('click', () => {
          if (grey && !accepted.has(b.id)) { noticeFor = b.id; pickerWrap.open = true; renderPicker(); return; }
          chosen = b.id; noticeFor = null; renderPicker();
        });
        picker.append(btn);
        if (grey && b.notice && (isOn || noticeFor === b.id)) picker.append(h('p.notice', b.notice));
        if (noticeFor === b.id && !accepted.has(b.id)) picker.append(h('div.btnrow', { style: { margin: '0 0 6px 18px' } },
          h('button.textbtn.strong', { type: 'button', onclick: () => { accepted.add(b.id); chosen = b.id; noticeFor = null; renderPicker(); } }, 'use it anyway, at my request'),
          h('button.textbtn', { type: 'button', onclick: () => { noticeFor = null; renderPicker(); } }, 'not now')));
      }
      picker.append(h('div.btnrow', { style: { marginTop: '6px' } },
        h('button.textbtn', { type: 'button', onclick: () => ctx.openSettings?.('connections') }, 'connections…')));
      const cur = byId(chosen);
      clear(chosenLabel).append(...(cur ? ['with ', nameOf(cur), ` · ${egressLabel(cur, settings)}`] : ['no model chosen']));
    };

    const load = async () => {
      try {
        const [r, s] = await Promise.all([api.backends(), api.get('/api/settings').catch(() => null)]);
        resp = r; settings = s;
        backends = r.backends || [];
        if (!byId(chosen)) chosen = initialChoice(r);
        renderPicker();
      } catch { clear(picker).append(h('p.empty', 'backends unavailable.')); }
    };
    load();
    window.addEventListener('anamnesis:backendschange', load);

    const titleOf = async (id) => {
      const i = store.index.get(id);
      if (i != null) return store.cloud.titles[i];
      try { return (await api.item(id)).item.title || id.slice(-6); } catch { return id.slice(-6).toLowerCase(); }
    };

    // the consent prompt: which items would leave, to where; send / answer here / don't
    const consentPrompt = (msg, ids, backend, resolve) => {
      const list = h('ul.list.consent-items');
      for (const id of ids) {
        const li = h('li', h('button.textbtn.keep-case', { type: 'button', onclick: () => ctx.open(id) }, '…'));
        titleOf(id).then((t) => { li.firstChild.textContent = t; });
        list.append(li);
      }
      const lb = localBackend();
      const box = h('div.consent', { role: 'group', 'aria-label': 'items that would leave this machine' },
        h('p', `${ids.length === 1 ? 'one note' : ids.length + ' notes'} you marked `, h('em', 'ask'), ' would go to ', backend ? nameOf(backend) : 'the model', ', which leaves this machine. nothing has been sent yet.'),
        list,
        h('div.btnrow',
          h('button.textbtn.strong', { type: 'button', onclick: () => done('approve') }, ids.length === 1 ? 'send it, for this chat' : 'send them, for this chat'),
          lb ? h('button.textbtn', { type: 'button', onclick: () => done('local') }, `answer here instead (${displayLabel(lb)})`) : null,
          h('button.textbtn', { type: 'button', onclick: () => done('deny') }, "don't send")),
        h('p.meta', 'each approval is logged; the egress ledger in settings → privacy shows what left.'));
      const done = (what) => { box.querySelectorAll('button').forEach((b) => { b.disabled = true; }); box.classList.add('answered'); resolve(what, lb); };
      msg.append(box);
      box.querySelector('button.strong')?.focus({ preventScroll: true });
      box.scrollIntoView({ block: 'nearest' });
    };

    const ask = async (text) => {
      text = text.trim();
      if (!text || busy) return;
      if (log.querySelector('.empty')) clear(log);
      log.append(h('div.chat-msg.you', h('div.who', 'you'), h('div.text', text)));
      await turn(text, withApprovals(consent, { message: text, backend: chosen || undefined }));
    };

    const turn = async (text, body) => {
      const textEl = h('div.text');
      const caret = h('span.caret', { 'aria-hidden': 'true' });
      const cites = h('div.cites');
      const usage = h('div.usage');
      const backend = byId(body.backend);
      const msg = h('div.chat-msg.assistant', h('div.who', backend ? ['assistant · ', nameOf(backend)] : 'assistant'), h('div', textEl, caret), cites, usage);
      log.append(msg);
      msg.scrollIntoView({ block: 'end' });
      clearBit(BIT.CITED);
      const cited = new Set();
      let consentEv = null;
      busy = new AbortController();
      send.hidden = true; stop.hidden = false;
      try {
        await streamChat(body, (ev, data) => {
          if (ev === 'token') { textEl.append(data.text || ''); if (msg.getBoundingClientRect().bottom > innerHeight - 40) msg.scrollIntoView({ block: 'end' }); }
          else if (ev === 'cited') {
            const idx = [];
            for (const id of data.ids || []) {
              if (cited.has(id)) continue;
              cited.add(id);
              const i = store.index.get(id);
              if (i != null) idx.push(i);
              const t = h('button.cite', { type: 'button', onclick: () => ctx.open(id) }, '· ' + (i != null ? store.cloud.titles[i] : id.slice(-6)));
              if (i == null) titleOf(id).then((x) => { t.textContent = '· ' + x; });
              cites.append(h('span.cite-row', t, gradeControl(text, id, ctx)));
            }
            setBit(BIT.CITED, idx, { exclusive: false, label: 'cited by the chat' });
            if (idx.length && store.view.page === 'brain') ctx.brain.focusOn([...cited].map((id) => store.index.get(id)).filter((i) => i != null), { soft: true });
          } else if (ev === 'usage') {
            usage.textContent = `${data.backend} · ${data.input} in · ${data.output} out${data.estimated ? ' · estimated' : ''}`;
          } else if (ev === 'done') {
            setChat(consent, data.chat_id);
            if (data.chat_id) { chatId = data.chat_id; distilBtn.disabled = false; }
          } else if (ev === 'consent_required') {
            consentEv = data || {};
          } else if (ev === 'error') {
            textEl.append(h('span.soft', ` (${data.message || 'error'})`));
          }
        }, busy.signal);
      } catch (e) {
        textEl.append(h('span.soft', e.name === 'AbortError' ? ' (stopped)' : ' (the model could not be reached)'));
      } finally {
        caret.remove();
        tidy(textEl, [...cited]);
        busy = null; send.hidden = false; stop.hidden = true;
        if (cited.size) cites.prepend(h('span.meta.cite-hint', 'relevant? tap 0–3'));
        if (!document.querySelector('.panel-chat')?.isConnected) { unread++; ctx.layout.render(); }
      }
      if (consentEv) {
        const r = onConsentRequired(consent, body, consentEv);
        if (r.kind === 'loop') { textEl.append(h('span.soft', '(the model still refuses these notes; nothing was sent.)')); return; }
        if (!textEl.textContent) msg.classList.add('waiting');
        consentPrompt(msg, r.ids, backend, (what, lb) => {
          if (what === 'approve') { const next = approve(consent); if (next) turn(text, next); }
          else if (what === 'local') { const next = deny(consent, lb.id); if (next) turn(text, next); }
          else { deny(consent); msg.append(h('p.meta', 'not sent. the question stays here.')); }
        });
      }
    };
    // ---- distil: notes in the user's own words, kept or dropped one by one ----
    const dailyToggle = () => {
      const on = !!settings?.distil?.daily;
      const box = h('input', { type: 'checkbox', checked: on, onchange: async () => {
        try { settings = await api.put('/api/settings', { distil: { daily: box.checked } }); }
        catch { box.checked = !box.checked; ctx.toast?.('the setting could not be saved.'); }
      } });
      return h('label.meta.distil-daily', box, ' also distil new chats once a day, in the background (same model rules: local first, an api only if you allowed background use)');
    };
    const distil = async () => {
      if (!chatId || distilBtn.disabled) return;
      distilBtn.disabled = true;
      const box = h('div.distil', { role: 'region', 'aria-label': 'distilled notes' }, h('p.meta', 'reading your messages…'));
      log.append(box);
      box.scrollIntoView({ block: 'nearest' });
      let out;
      try { out = await api.distil(chatId); }
      catch (e) { clear(box).append(h('p.meta', e.serverMessage || 'distil did not work this time.')); distilBtn.disabled = false; return; }
      distilBtn.disabled = false;
      clear(box).append(h('p.meta.distil-summary', distilSummary(out)));
      const ps = out.proposals || [];
      const answered = ps.map(() => false);
      const cards = [];
      const decide = async (i, keep) => {
        if (answered[i]) return;
        const p = ps[i], card = cards[i];
        card.querySelectorAll('button').forEach((b) => { b.disabled = true; });
        let r;
        try { r = await api.distilDecide(p.id, keep); }
        catch (e) { card.querySelectorAll('button').forEach((b) => { b.disabled = false; }); ctx.toast?.(e.serverMessage || 'that did not work.'); return; }
        answered[i] = true;
        card.classList.add('answered', keep ? 'kept' : 'dropped');
        const row = card.querySelector('.btnrow');
        clear(row).append(keep
          ? h('span.meta', 'kept as a note · ', h('button.textbtn', { type: 'button', onclick: () => ctx.open(r.item.id) }, 'open it'))
          : h('span.meta', 'dropped'));
        const j = nextOpen(answered, i);
        if (j >= 0) cards[j].focus({ preventScroll: false });
        else { box.append(h('p.meta', 'all answered.')); input.focus({ preventScroll: true }); }
      };
      ps.forEach((p, i) => {
        const card = h('div.distil-card', { role: 'group', tabindex: '0', 'aria-label': `${kindLabel(p.kind)}: ${p.title}. k keeps, d drops.` },
          h('div.who', kindLabel(p.kind)),
          h('div.distil-title.keep-case', p.title),
          h('blockquote.keep-case', p.quote),
          h('div.meta', titleBy(p)),
          h('div.btnrow',
            h('button.textbtn.strong', { type: 'button', onclick: () => decide(i, true) }, 'keep'),
            h('button.textbtn', { type: 'button', onclick: () => decide(i, false) }, 'drop')));
        card.addEventListener('keydown', (e) => {
          if (e.target !== card) return; // the buttons handle their own keys
          const a = keyAction(e.key);
          if (!a) return;
          e.preventDefault();
          if (a === 'keep' || a === 'drop') decide(i, a === 'keep');
          else cards[a === 'next' ? Math.min(i + 1, cards.length - 1) : Math.max(i - 1, 0)].focus();
        });
        cards.push(card);
        box.append(card);
      });
      box.append(dailyToggle());
      if (cards.length) cards[0].focus({ preventScroll: true });
      box.scrollIntoView({ block: 'nearest' });
    };

    form.addEventListener('submit', (e) => { e.preventDefault(); const t = input.value; input.value = ''; ask(t); });
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); form.requestSubmit(); } });
    on('chat-ask', (t) => { ask(t); });
  },
  shown() { unread = 0; },
};
