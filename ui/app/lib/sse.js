// Incremental text/event-stream parser. Feed it chunks; it returns complete
// events and keeps the unfinished tail. Pure, so it is unit-tested in node.
export function parseSSE(buf) {
  const events = [];
  let cut;
  while ((cut = buf.search(/\r?\n\r?\n/)) >= 0) {
    const block = buf.slice(0, cut);
    buf = buf.slice(cut).replace(/^\r?\n\r?\n/, '');
    let ev = 'message';
    const data = [];
    for (const line of block.split(/\r?\n/)) {
      if (line.startsWith('event:')) ev = line.slice(6).trim();
      else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
    }
    if (!data.length) continue;
    let parsed;
    try { parsed = JSON.parse(data.join('\n')); } catch { parsed = { text: data.join('\n') }; }
    events.push([ev, parsed]);
  }
  return { events, rest: buf };
}
