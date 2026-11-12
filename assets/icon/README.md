# the anamnesis icon

six small linked rings, three lit with stipple, as if being recalled. it comes from the aphes.ist design system (the aphesis repository, `system/assets/Icons/`), where it is drawn with `Aphesis.icon('anamnesis', state)`.

- `anamnesis.svg`: still, at rest, in rubedo `#ab0d19` (the light theme's accent). the source of every app icon.
- `anamnesis-chosen.svg`: still, its stipple gathered.
- `anamnesis-dark.svg`, `anamnesis-light.svg`: animated, transparent, in each theme's accent; still under `prefers-reduced-motion`.

`packaging/icons/make-icons.py` renders the desktop app's icons (`crates/anamnesis-desktop/icons/`), the windows installer's `packaging/windows/anamnesis.ico` and the ui's favicon (`ui/app/favicon.svg`) from `anamnesis.svg`.

the icon, like the φ mark, belongs to the owner of aphes.ist and is not licensed for use by forks: a fork should draw its own.
