# site

the landing page of [anamnesis](https://github.com/aphesist/anamnesis), served at [anamnesis.aphes.ist](https://anamnesis.aphes.ist) by github pages straight from this branch (settings → pages → deploy from a branch → `site`, `/ (root)`).

- plain static files, no build step: `index.html`, `style.css`, `main.js`.
- `hero/`: the turning demo brain (`brain.js`, `brain-data.bin`, static fallbacks). `hero/build/extract.py` repacks the data from a checkout of `main`: `python3 hero/build/extract.py ../anamnesis/demo/transcendentalist/snapshot`.
- `vendor/`: pinned copies of the aphes.ist design system and lab kit (the owner's, not under anamnesis's apache-2.0 licence; see their notices).
- `tools/make_icons.py`: the favicons.

preview: `python3 -m http.server` here, then open http://127.0.0.1:8000/.
