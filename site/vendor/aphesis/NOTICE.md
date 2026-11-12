# aphes.ist design system: not part of the anamnesis licence

The files in this folder are aphesist's: the aphes.ist design system and its runtime, from the
[aphesis](https://github.com/aphesist/aphesis) repository. Their owner uses them here, on anamnesis's own page.
They are **not** covered by anamnesis's Apache-2.0 licence, and a fork of anamnesis may not use them to present itself.

Nothing on this page loads anything from aphes.ist at run time: every file is here, pinned.

| file | what | from (aphesis repository) | terms |
|---|---|---|---|
| `bundle.js` | the runtime, `window.Aphesis`: the fog, the brush and stipple, the seal, the eye's theme wave, the cursor, the tooltip, the letters, the typist, the scroll disturbance | `design-system` @ `a1b9de4`, `system/components/bundle.js` (unchanged) | the owner's; all rights reserved |
| `bundle.css` | the components and the browser defaults (scrollbars, selection, focus, fields…) | `design-system` @ `a1b9de4`, `system/components/bundle.css`, with its first `@import` of Google Fonts removed (the faces are in `fonts.css`) | the owner's; all rights reserved |
| `tokens.json` | the tokens | `design-system` @ `a1b9de4`, `system/tokens.json` (unchanged) | the owner's; all rights reserved |
| `tokens.css` | the tokens as custom properties | generated from `tokens.json` as aphes.ist's build does (`site/build.py`, `tokens_css`, on `site` @ `6b95ac8`) | as above |
| `fonts.css` | the two `@font-face` rules | as aphes.ist's build declares them (`site/build.py`, `faces`) | as above |
| `favicon.js` | the tab's icon: the eye, open and closed, drawn with the runtime's brush and stipple | `site` @ `6b95ac8`, `site/src/favicon.js` (unchanged) | the owner's; all rights reserved |
| `phi-closed.d` | the closed Φ as the seal engine draws it in the corner (SVG path data), the source of `favicon.ico` and the touch icons (`site/tools/make_icons.py`) | the output of `mark` @ `074d4d9`, `engine/model.js` + `engine/core.js`, with the parameters of aphes.ist's `site/tools/make_icons.py` | the φ is a trademark of aphesist ([TRADEMARKS.md](https://github.com/aphesist/aphesis/blob/design-system/TRADEMARKS.md)) |
| `fonts/AphesisScript-VF.woff2` | Aphesis Script, the display face | `design-system` @ `a1b9de4`, `system/fonts/` | [SIL Open Font License 1.1](fonts/AphesisScript-OFL.txt), Reserved Font Name "Aphesis Script" |
| `fonts/SpecialElite-latin*.woff2` | Special Elite, the text face (latin and latin-ext subsets) | `site` @ `6b95ac8`, `site/vendor/` | [Apache License 2.0](fonts/SpecialElite-LICENSE.txt), © 2010 Brian J. Bonislawsky DBA Astigmatic |

The φ and the wordmarks set in Aphesis Script (`aφesis`, and `anamnesis` as this page sets it) are trademarks of
aphesist and are not licensed by the OFL or by Apache-2.0.

Full commits: design-system `a1b9de48661f37041f01b2002b785c20ee6a95b1`, site `6b95ac8a0791fe938ff77ff634add6b0c0dcb6a1`,
mark `074d4d99e495af6f682b41e6c239e7bafe6f25ed` (all of 2026-11-11). To update, copy the same files from newer commits
and change the table.
