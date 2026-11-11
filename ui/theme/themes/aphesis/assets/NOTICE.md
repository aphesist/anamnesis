# aphesis theme assets: not part of the anamnesis licence

This folder is where the default `aphesis` theme looks for the aphes.ist identity assets. They belong to the owner of aphes.ist (github.com/aphesist/aphesis) and are **not** covered by anamnesis's Apache-2.0 licence.

| file | what | terms |
|---|---|---|
| `AphesisScript-VF.woff2` | Aphesis Script, the variable display face | to be licensed by its author under the SIL Open Font License 1.1 with the Reserved Font Name "Aphesis Script". **Pending:** no licence file has been published upstream yet, so until it is, the font is all rights reserved and must not be committed to this repository or redistributed in builds. |
| `phi.svg` | the φ mark | a trademark of the owner of aphes.ist. Not covered by Apache-2.0 (see its §6). Do not use it to identify a modified or forked product. |
| the wordmark | `aφesis` / the anamnesis wordmark set in Aphesis Script | a trademark of the owner, as above. No file is fetched; the theme sets it in the display face. |

## how they get here

`node ui/theme/scripts/fetch-aphesis-assets.mjs` downloads them from the owner's public repository (also run by `npm install` inside `ui/theme`). Everything in this folder except this notice is ignored by git.

Without them the theme still works: the display face falls back to Special Elite (Apache-2.0, bundled in `ui/theme/fonts/`) and then the system serif, and the mark falls back to `ui/theme/marks/phi-placeholder.svg`, a stipple φ drawn for anamnesis.

## for forks

Forks must swap the mark and the face: set `--mark`, `--wordmark` and `--font-display` in your own theme (see `ui/theme/README.md`), and do not ship these files under the name Aphesis.

## for the owner

Once the OFL-1.1 licence (with the Reserved Font Name) is published next to the font upstream, the font binary may be committed here together with its `OFL.txt`, and this notice and `.gitignore` updated. The φ and the wordmark stay trademarks either way; a `TRADEMARKS.md` at the repository root would make that explicit.
