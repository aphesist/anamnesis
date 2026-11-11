# theming

everything you see is themeable: colours, fonts, the logo, motion, the cursor, and where each panel sits. the ui reads only a small set of css custom properties ("tokens"), so a theme is a set of values, not a fork of the ui.

the full reference (the token list, the `theme.js` api, the settings panel, the layout schema, how to build a theme) is **[ui/theme/README.md](../ui/theme/README.md)**. this page is the short version.

## in the app

**more → settings & theme** opens the settings panel:

| section | what you can change |
|---|---|
| theme | the theme (`aphesis`, `paper`, `mono`, or one you imported) and the mode: system, light or dark |
| colour | any colour role or brain colour, for one mode or both |
| type | a font per role (`display`, `text`, `data`), uploaded files included, and the overall type size |
| motion | full or reduced motion, the custom cursor disc, hidden or system scrollbars, and "ask again" for the first-launch question |
| mark | upload your own mark and wordmark (svg or png) |
| layout | show or hide panels, which start open, which side they sit on, their order, presets (`default`, `minimal`) |
| keep | export your look as a json file, import one, reset |

choices are stored in the browser and, when the app is served by `anamnesis`, merged into the server settings (`theme`, `motion`). uploaded fonts and logos stay on the device.

## motion

the default motion is heavy: the brain turns, the stipple fog drifts, views morph into each other. on first launch the app asks once whether to keep it or calm it, pre-selecting your system's reduced-motion setting, and you can change it later in settings (or from **more → reduce motion**). with reduced motion, `--motion` is `0` and the ui drops its animations.

## tokens

the contract ([contracts.md](contracts.md#theme-tokens-css-custom-properties-on-root)):

- colour roles: `--bg`, `--ink`, `--ink-soft`, `--line`, `--fog`, `--accent`, `--accent-text`, `--seal`
- the brain: `--dot`, `--dot-dim`, `--dot-hot`, `--thread`, `--region-label`
- type: `--font-display`, `--font-text`, `--font-data`, `--type-scale`
- motion: `--ease`, `--dur-short`, `--dur-mid`, `--dur-long`, `--motion`
- spacing: `--space-1` … `--space-9` (fibonacci on 4 px)
- marks: `--mark`, `--wordmark`

tokens are written in the [w3c design tokens format](https://www.w3.org/community/reports/design-tokens/CG-FINAL-format-20251028/), compiled to css by `node ui/theme/scripts/build-tokens.mjs`. the webgl renderer re-reads the colour tokens on every `anamnesis:themechange`, so the point cloud follows the theme. the theme tests check that text keeps at least 4.5:1 contrast (7:1 for body ink) in both modes.

## making a theme

copy `ui/theme/themes/paper/` to a new folder, change the tokens that differ, rebuild, done. or, without the repository: export your look from settings, edit the json, import it. step by step in [ui/theme/README.md](../ui/theme/README.md#making-a-theme).

## layout

panels are placed by a layout object (`ui/theme/layouts/default.json`, schema in `layout.schema.json`): each panel has a region (`left`, `right`, `bottom`, `overlay`), an order, whether it starts open and whether it is visible. the panels are `sidebar-left`, `view-right`, `legend-bottom-left`, `when-you-wrote`, `timeline-bottom`, `chat`, `review`, `threads`, `eval` and `skills`. rearrange them in settings → layout.

## the aphesis theme and its assets

the default theme reproduces the aphes.ist design system: a dark oklch neutral scale, rubedo as the single accent, lowercase, brush strokes and stipple, a difference-blend cursor disc and hidden scrollbars, fibonacci spacing and one settling easing.

its identity assets are **not part of this repository and not under the apache licence**:

| asset | status |
|---|---|
| Aphesis Script (the display face) | to be released by its author under the SIL Open Font License 1.1 with the reserved font name "Aphesis Script". until that licence is published it is all rights reserved: it is fetched at install time, never committed, never bundled. |
| the φ mark and the wordmark | trademarks of the owner of aphes.ist. the apache licence grants no trademark rights. |

`npm install` in `ui/theme` (or `node ui/theme/scripts/fetch-aphesis-assets.mjs`) fetches them into `ui/theme/themes/aphesis/assets/`, which git ignores; `ANAMNESIS_SKIP_ASSET_FETCH=1` skips it. without them the theme still works: the display face falls back to **Special Elite** (apache-2.0, © 2010 Astigmatic, bundled in `ui/theme/fonts/`) and then the system serif, and the mark falls back to a stipple φ placeholder drawn for anamnesis.

**if you fork anamnesis**, set your own `--mark`, `--wordmark` and `--font-display`, and do not ship the aphesis assets under that name. details: [ui/theme/themes/aphesis/assets/NOTICE.md](../ui/theme/themes/aphesis/assets/NOTICE.md) and [NOTICE](../NOTICE).
