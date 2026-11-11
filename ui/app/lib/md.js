// A deliberately small markdown renderer for the reader and the editor preview.
// Escapes everything first, then adds headings, quotes, lists, emphasis, code and [[wikilinks]].
import { esc } from './dom.js';

function inline(s) {
  return s
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|[\s(])\*([^*]+)\*/g, '$1<em>$2</em>')
    .replace(/\[\[([^\]]+)\]\]/g, '<a href="#" class="wikilink" data-title="$1">$1</a>')
    .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" rel="noopener noreferrer" target="_blank">$1</a>');
}

export function renderMarkdown(src) {
  const lines = esc(src || '').split(/\r?\n/);
  const out = [];
  let list = null, para = [], quote = [], code = null;
  const flushPara = () => { if (para.length) { out.push('<p>' + inline(para.join(' ')) + '</p>'); para = []; } };
  const flushList = () => { if (list) { out.push(`<${list.t}>` + list.items.map((i) => '<li>' + inline(i) + '</li>').join('') + `</${list.t}>`); list = null; } };
  const flushQuote = () => { if (quote.length) { out.push('<blockquote>' + inline(quote.join(' ')) + '</blockquote>'); quote = []; } };
  const flush = () => { flushPara(); flushList(); flushQuote(); };
  for (const line of lines) {
    if (code) { if (/^```/.test(line)) { out.push('<pre><code>' + code.join('\n') + '</code></pre>'); code = null; } else code.push(line); continue; }
    if (/^```/.test(line)) { flush(); code = []; continue; }
    let m;
    if ((m = line.match(/^(#{1,4})\s+(.*)$/))) { flush(); const n = Math.min(6, m[1].length + 1); out.push(`<h${n}>${inline(m[2])}</h${n}>`); continue; }
    if ((m = line.match(/^&gt;\s?(.*)$/))) { flushPara(); flushList(); quote.push(m[1]); continue; }
    if ((m = line.match(/^\s*[-*]\s+(.*)$/))) { flushPara(); flushQuote(); if (!list || list.t !== 'ul') { flushList(); list = { t: 'ul', items: [] }; } list.items.push(m[1]); continue; }
    if ((m = line.match(/^\s*\d+[.)]\s+(.*)$/))) { flushPara(); flushQuote(); if (!list || list.t !== 'ol') { flushList(); list = { t: 'ol', items: [] }; } list.items.push(m[1]); continue; }
    if (!line.trim()) { flush(); continue; }
    flushList(); flushQuote(); para.push(line);
  }
  if (code) out.push('<pre><code>' + code.join('\n') + '</code></pre>');
  flush();
  return out.join('\n');
}
