// Brush-drawn line icons (original paths, drawn for anamnesis). 18×18 viewBox.
const P = {
  brain: 'M9 3.2c-1.6-1.3-4.4-.6-4.6 1.6-1.9.3-2.6 2.6-1.4 3.9-1 1.5-.1 3.6 1.7 3.7.3 1.9 2.5 2.8 4.3 1.7M9 3.2c1.6-1.3 4.4-.6 4.6 1.6 1.9.3 2.6 2.6 1.4 3.9 1 1.5.1 3.6-1.7 3.7-.3 1.9-2.5 2.8-4.3 1.7M9 3.4v11M6.2 6.6c.9.2 1.7.9 1.9 1.8M11.9 6.4c-.9.3-1.6 1-1.8 1.9M5.4 10.4c1-.4 2-.2 2.7.5M12.6 10.5c-1-.4-2-.2-2.6.6',
  notes: 'M4.2 3.6c3.1-.2 6.4-.1 9.4.1M4 7.2c2.4-.1 5.3-.2 9.2 0M4.1 10.8c2.9-.1 5.7 0 8.1.1M4.3 14.3c1.6-.1 3.2-.1 4.6 0',
  write: 'M3.6 14.6c2.3-.5 3.9-1.6 5.5-3.3l5.1-6c.7-.9.1-2-1-1.7-.6.2-1 .6-1.4 1.1L6.7 10.9c-1.3 1.4-2.3 2.4-3.1 3.7M11 5.6l1.6 1.4M3.4 15.6c3.2-.2 7.6-.2 11.2 0',
  today: 'M9 5.4c2.1-.1 3.7 1.6 3.6 3.7-.1 2-1.7 3.5-3.7 3.4-2-.1-3.4-1.7-3.4-3.6.1-2 1.6-3.5 3.5-3.5M9 1.8v1.6M9 14.6v1.6M1.9 9h1.6M14.5 9h1.6M3.9 3.9l1.1 1.1M13 13l1.1 1.1M3.9 14.1 5 13M13 5l1.1-1.1',
  add: 'M9.1 3.4c-.2 3.7-.1 7.4.1 11.2M3.5 9.1c3.6-.2 7.4-.2 11.1.1',
  more: '',
  ask: 'M7.9 3.3c2.6-.1 4.6 2 4.5 4.6-.1 2.5-2.2 4.4-4.7 4.3-2.4-.1-4.2-2.2-4.1-4.6.1-2.4 2-4.3 4.3-4.3M11.3 11.4c1.3 1.2 2.4 2.4 3.6 3.8',
};
export function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 18 18');
  svg.setAttribute('aria-hidden', 'true');
  svg.classList.add('icon');
  if (name === 'more') {
    for (const x of [4.2, 9, 13.8]) {
      const c = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
      c.setAttribute('cx', x); c.setAttribute('cy', 9.2 + (x - 9) * 0.04); c.setAttribute('r', 1.05);
      svg.append(c);
    }
    return svg;
  }
  const p = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  p.setAttribute('d', P[name] || '');
  svg.append(p);
  return svg;
}
