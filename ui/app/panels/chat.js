// Chat: streams tokens from POST /api/chat (server-sent events), lights the
// cited dots in the brain, and lets the user pick a backend with its terms
// status and notice from /api/backends.
import { h, clear } from '../lib/dom.js';
import { store, on, BIT, setBit, clearBit } from '../lib/store.js';
import { api, streamChat } from '../lib/api.js';

let unread = 0;
export default {
  id: 'chat',
  title: 'chat',
  badge: () => unread || '',
  mount(el, ctx) {
    el.classList.add('chat');
    let backends = [], chosen = null, chatId = null, busy = null, accepted = new Set();
    const picker = h('div.backend-picker', { role: 'radiogroup', 'aria-label': 'model' });
    const pickerWrap = h('details.sect', h('summary', 'model'), picker);
    const chosenLabel = h('span.meta', '');
    const log = h('div.chat-log', { 'aria-live': 'polite', 'aria-relevant': 'additions' });
    const input = h('textarea', { placeholder: 'ask your notes something…', 'aria-label': 'message', rows: 3 });
    const send = h('button.textbtn.strong', { type: 'submit' }, 'send');
    const stop = h('button.textbtn', { type: 'button', hidden: true, onclick: () => busy?.abort() }, 'stop');
    const form = h('form.chat-form', input, h('div.btnrow', { style: { justifyContent: 'space-between' } }, h('div.btnrow', send, stop), chosenLabel));
    log.append(h('p.empty', 'ask anything. answers cite your own notes, and the cited dots light up in the brain.'));
    el.append(pickerWrap, log, form);

    const renderPicker = () => {
      clear(picker);
      for (const b of backends) {
        const isOn = chosen === b.id;
        const btn = h('button.be', { type: 'button', role: 'radio', 'aria-checked': String(isOn), disabled: !b.available, title: b.status || '' },
          h('span', isOn ? '●' : '○'),
          h('span', h('span.brand-label', b.kind === 'chatgpt_signin' ? b.label : b.label.toLowerCase()), h('span.meta', ` · ${b.status || (b.available ? 'ready' : 'unavailable')}`)),
          h(`span.terms.${b.terms === 'grey' ? 'grey' : 'ok'}`, b.terms === 'grey' ? 'terms: unclear' : 'terms: allowed'));
        btn.addEventListener('click', () => {
          if (b.terms === 'grey' && !accepted.has(b.id)) { renderNotice(b); return; }
          chosen = b.id; renderPicker();
        });
        picker.append(btn);
        if (b.terms === 'grey' && b.notice && (isOn || noticeFor === b.id)) picker.append(h('p.notice', b.notice));
        if (noticeFor === b.id && !accepted.has(b.id)) picker.append(h('div.btnrow', { style: { margin: '0 0 6px 18px' } },
          h('button.textbtn.strong', { type: 'button', onclick: () => { accepted.add(b.id); chosen = b.id; noticeFor = null; renderPicker(); } }, 'use it anyway, at my request'),
          h('button.textbtn', { type: 'button', onclick: () => { noticeFor = null; renderPicker(); } }, 'not now')));
      }
      const cur = backends.find((b) => b.id === chosen);
      chosenLabel.textContent = cur ? `with ${cur.kind === 'chatgpt_signin' ? cur.label : cur.label.toLowerCase()}${cur.kind === 'local' ? ' · stays on this machine' : ' · leaves this machine, logged'}` : 'no model chosen';
    };
    let noticeFor = null;
    const renderNotice = (b) => { noticeFor = b.id; pickerWrap.open = true; renderPicker(); };

    const load = async () => {
      try {
        const r = await api.backends();
        backends = r.backends || [];
        chosen = r.routing?.chat || backends.find((b) => b.available && b.terms !== 'grey')?.id || null;
        renderPicker();
      } catch { picker.append(h('p.empty', 'backends unavailable.')); }
    };
    load();

    const ask = async (text) => {
      text = text.trim();
      if (!text || busy) return;
      if (log.querySelector('.empty')) clear(log);
      log.append(h('div.chat-msg.you', h('div.who', 'you'), h('div.text', text)));
      const textEl = h('div.text');
      const caret = h('span.caret', { 'aria-hidden': 'true' });
      const cites = h('div.cites');
      const usage = h('div.usage');
      const msg = h('div.chat-msg.assistant', h('div.who', 'assistant'), h('div', textEl, caret), cites, usage);
      log.append(msg);
      msg.scrollIntoView({ block: 'end' });
      clearBit(BIT.CITED);
      const cited = new Set();
      busy = new AbortController();
      send.hidden = true; stop.hidden = false;
      try {
        await streamChat({ chat_id: chatId || undefined, message: text, backend: chosen || undefined }, (ev, data) => {
          if (ev === 'token') { textEl.append(data.text || ''); if (msg.getBoundingClientRect().bottom > innerHeight - 40) msg.scrollIntoView({ block: 'end' }); }
          else if (ev === 'cited') {
            const idx = [];
            for (const id of data.ids || []) {
              if (cited.has(id)) continue;
              cited.add(id);
              const i = store.index.get(id);
              if (i != null) idx.push(i);
              cites.append(h('button', { type: 'button', onclick: () => ctx.open(id) }, '· ' + (i != null ? store.cloud.titles[i] : id.slice(-6))));
            }
            setBit(BIT.CITED, idx, { exclusive: false, label: 'cited by the chat' });
            if (idx.length && store.view.page === 'brain') ctx.brain.focusOn([...cited].map((id) => store.index.get(id)).filter((i) => i != null));
          } else if (ev === 'usage') {
            usage.textContent = `${data.backend} · ${data.input} in · ${data.output} out${data.estimated ? ' · estimated' : ''}`;
          } else if (ev === 'done') {
            chatId = data.chat_id || chatId;
          } else if (ev === 'error') {
            textEl.append(h('span.soft', ` (${data.message || 'error'})`));
          }
        }, busy.signal);
      } catch (e) {
        textEl.append(h('span.soft', e.name === 'AbortError' ? ' (stopped)' : ' (the model could not be reached)'));
      } finally {
        caret.remove();
        busy = null; send.hidden = false; stop.hidden = true;
        if (!document.querySelector('.panel-chat')?.isConnected) { unread++; ctx.layout.render(); }
      }
    };
    form.addEventListener('submit', (e) => { e.preventDefault(); const t = input.value; input.value = ''; ask(t); });
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); form.requestSubmit(); } });
    on('chat-ask', (t) => { ask(t); });
  },
  shown() { unread = 0; },
};
