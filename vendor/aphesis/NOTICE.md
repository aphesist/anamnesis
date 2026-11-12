# aphes.ist design system: not part of the anamnesis licence

The files in this folder, and the lab kit beside it (`../lab-kit-v1/`), are aphesist's: the aphes.ist design system and its runtime, from the
[aphesis](https://github.com/aphesist/aphesis) repository. Their owner uses them here, on anamnesis's own page.
They are **not** covered by anamnesis's Apache-2.0 licence, and a fork of anamnesis may not use them to present itself.

Nothing on this page loads anything from aphes.ist at run time: every file is here, pinned.

| file | what | from (aphesis repository) | terms |
|---|---|---|---|
| `bundle.js` | the runtime, `window.Aphesis`: the fog, the brush and stipple, the icons (anamnesis's among them), the eye's theme wave, the cursor, the tooltip, the letters, the scroll disturbance | `design-system` @ `3bd45c5`, `system/components/bundle.js` (unchanged) | the owner's; all rights reserved |
| `bundle.css` | the components and the browser defaults (scrollbars, selection, focus, fields…) | `design-system` @ `3bd45c5` (the same as at `a1b9de4`), `system/components/bundle.css`, with its first `@import` of Google Fonts removed (the faces come from the lab kit's `lab.css`) | the owner's; all rights reserved |
| `tokens.json` | the tokens | `design-system` @ `3bd45c5`, `system/tokens.json` (unchanged) | the owner's; all rights reserved |
| `tokens.css` | the tokens as custom properties | generated from `tokens.json` as aphes.ist's build does (`site/build.py`, `tokens_css`, on `site` @ `6b95ac8`) | as above |
| `anamnesis.svg` | anamnesis's icon (six small linked rings, three lit with stipple), the source of `favicon.svg`, `favicon.ico` and the touch icons (`site/tools/make_icons.mjs`) | `design-system` @ `85af21c`, `system/assets/Icons/anamnesis.svg` (unchanged) | the owner's mark; all rights reserved |
| `../lab-kit-v1/` | the lab kit 1.1.0: `lab.css` (the two faces, the tokens, the receding header and bands), `lab.js` (it measures the sticky header into `--lab-header-h`), and `fonts/`: Aphesis Script ([SIL Open Font License 1.1](../lab-kit-v1/fonts/AphesisScript-OFL.txt), Reserved Font Name "Aphesis Script") and Special Elite ([Apache License 2.0](../lab-kit-v1/fonts/SpecialElite-LICENSE.txt), © 2010 Brian J. Bonislawsky DBA Astigmatic) | `design-system` @ `3bd45c5`, `system/lab-kit/v1/` (unchanged) | the owner's; the fonts under their own licences |

The φ and the wordmarks set in Aphesis Script (`aφesis`, and `anamnesis` as this page sets it) are trademarks of
aphesist and are not licensed by the OFL or by Apache-2.0.

Full commits: design-system `3bd45c553e8fc4a8fe652c9c9fd6557de8ff0634` (lab kit 1.1 and the project icons), `85af21cfcfdb7c796e3e77332b421920c28cd9da` for the icon, site `6b95ac8a0791fe938ff77ff634add6b0c0dcb6a1` (2026-11-11 and 12). To update, copy the same files from newer commits
and change the table.
