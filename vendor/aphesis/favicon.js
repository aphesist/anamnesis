/* the tab's icon: the eye, as SVG. The same brush and stipple as the eye in the corner, with a heavier hand for 16 and
   32 px: thicker strokes and an iris of a few large cells. Rubedo (the dark theme's where the browser is dark) on a
   transparent ground. The shell swaps the two as the page is seen or left; the build runs this too (with node), so
   the open eye is in the page's head before any script runs. favicons(Aphesis, {light, dark}) -> {open, closed} */
function favicons(A, red) {
  const open = A.brush(A.arc(12, 15.6, 10, -158, -22, 40, 7.6), 2.1, 31) + A.brush(A.arc(12, 8.4, 10, 22, 158, 40, 7.6), 2.1, 33) +
    A.stipple(A.inC(12, 12, 2.9), 17, 150, 1.45);
  let closed = A.brush(A.arc(12, 8, 10, 25, 155, 40, 6.4), 2.3, 37);
  [-0.55, 0, 0.55].forEach((k, i) => { const x = 12 + k * 10.4; closed += A.brush(A.line(x, 14.4 - Math.abs(k) * 1.2, x + k * 2.2, 18.8 - Math.abs(k), 10), 1.7, 41 + i, 0.4, 0.05); });
  const svg = (d) => 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="1 1 22 22"><style>path{fill:' + red.light +
    '}@media (prefers-color-scheme:dark){path{fill:' + red.dark + '}}</style><path d="' + d + '"/></svg>');
  return { open: svg(open), closed: svg(closed) };
}
