# aphesis theme assets: not part of the anamnesis licence

These are the aphes.ist identity assets used by the default `aphesis` theme. They belong to the owner of aphes.ist ([github.com/aphesist/aphesis](https://github.com/aphesist/aphesis)) and are **not** covered by anamnesis's Apache-2.0 licence.

| file | what | terms |
|---|---|---|
| `AphesisScript-VF.woff2` | Aphesis Script, the variable display face | [SIL Open Font License 1.1](OFL.txt), Reserved Font Name "Aphesis Script". Free to use, embed, bundle and modify; a modified version must not be called "Aphesis Script". |
| `phi.svg` | the φ mark | a trademark of the owner of aphes.ist ([TRADEMARKS.md](https://github.com/aphesist/aphesis/blob/design-system/TRADEMARKS.md)). Not covered by Apache-2.0 (see its §6) or by the OFL. anamnesis may display it in this theme as shipped. |
| the wordmark | `aφesis` set in Aphesis Script | a trademark of the owner, as above. No file; the theme sets it in the display face. |

`fetched.json` records where each file came from and its sha256. `node ui/theme/scripts/fetch-aphesis-assets.mjs --force` refreshes them from the owner's repository.

## for forks

You may keep and modify the font under the OFL (renamed if you change it). You may not use the φ or the wordmark to identify your fork: set `--mark`, `--wordmark` and, if you like, `--font-display` in your own theme (see `ui/theme/README.md`). Without `phi.svg` the theme falls back to `ui/theme/marks/phi-placeholder.svg`, a stipple φ drawn for anamnesis.
