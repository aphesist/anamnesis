// The default panel layout, used only when the theming package (ui/theme) is
// not present. It mirrors ui/theme/layouts/default.json and follows its schema
// (ui/theme/layouts/layout.schema.json); with the theme loaded, ui/theme/layout.js
// is the source of truth and this file is ignored.
export const DEFAULT_LAYOUT = {
  "version": 1,
  "id": "default",
  "name": "default",
  "description": "the reference arrangement: nav on the left, view on the right, legend bottom-left, time along the bottom; everything else opens on demand.",
  "panels": [
    {
      "id": "sidebar-left",
      "label": "nav",
      "region": "left",
      "order": 0,
      "open": true,
      "visible": true,
      "size": 72,
      "sections": [
        "ask-or-search",
        "brain",
        "notes",
        "write",
        "today",
        "add",
        "more",
        "mini-brain"
      ]
    },
    {
      "id": "view-right",
      "label": "view",
      "region": "right",
      "order": 0,
      "open": true,
      "visible": true,
      "size": 320,
      "sections": [
        "form",
        "order",
        "table",
        "groups",
        "show",
        "motion",
        "walkthrough",
        "demo-brain",
        "reset"
      ]
    },
    {
      "id": "when-you-wrote",
      "label": "when you wrote",
      "region": "right",
      "order": 1,
      "open": false,
      "visible": true,
      "size": 320,
      "sections": [
        "activity",
        "lifecycle",
        "calendar",
        "speakers"
      ]
    },
    {
      "id": "review",
      "label": "review",
      "region": "right",
      "order": 2,
      "open": false,
      "visible": true,
      "size": 320,
      "sections": [
        "due",
        "link-suggestions",
        "open-questions",
        "quarantine"
      ]
    },
    {
      "id": "threads",
      "label": "threads",
      "region": "right",
      "order": 3,
      "open": false,
      "visible": true,
      "size": 320
    },
    {
      "id": "legend-bottom-left",
      "label": "function (anatomy)",
      "region": "overlay",
      "order": 0,
      "open": true,
      "visible": true,
      "anchor": "bottom-left"
    },
    {
      "id": "timeline-bottom",
      "label": "time",
      "region": "bottom",
      "order": 0,
      "open": true,
      "visible": true,
      "size": 96
    },
    {
      "id": "chat",
      "label": "chat",
      "region": "overlay",
      "order": 1,
      "open": false,
      "visible": true,
      "anchor": "right",
      "size": 420
    },
    {
      "id": "eval",
      "label": "evaluation",
      "region": "overlay",
      "order": 2,
      "open": false,
      "visible": true,
      "anchor": "center"
    },
    {
      "id": "skills",
      "label": "skills",
      "region": "overlay",
      "order": 3,
      "open": false,
      "visible": true,
      "anchor": "center"
    }
  ]
};
