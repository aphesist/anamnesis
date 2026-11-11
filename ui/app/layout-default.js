// The default panel layout. Mirrors ui/theme/layouts/default.json (owned by the
// theming package); used when that file is missing. Shape:
//   { version, panels: [{ id, area: left|right|bottom|overlay|hidden, order, open, slot? }] }
// Right and bottom are tabbed docks (one panel open at a time); overlay panels
// sit on the brain at a named slot (bottom-left, bottom).
export const DEFAULT_LAYOUT = {
  version: 1,
  panels: [
    { id: 'sidebar', area: 'left', order: 0, open: true },
    { id: 'view', area: 'right', order: 0, open: true },
    { id: 'chat', area: 'right', order: 1, open: false },
    { id: 'threads', area: 'right', order: 2, open: false },
    { id: 'review', area: 'right', order: 3, open: false },
    { id: 'skills', area: 'right', order: 4, open: false },
    { id: 'eval', area: 'right', order: 5, open: false },
    { id: 'when', area: 'bottom', order: 0, open: false },
    { id: 'legend', area: 'overlay', slot: 'bottom-left', order: 0, open: true },
    { id: 'time', area: 'overlay', slot: 'bottom', order: 1, open: true },
  ],
};
