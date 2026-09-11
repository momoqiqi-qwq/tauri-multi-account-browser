# v10 changes

- Moved the floating Refresh/Home controls out of remote account pages and into a host-side browser action dock. The dock is rendered by the Tauri/Vue shell and occupies its own layout region, so it appears immediately and does not depend on a remote page refresh or injected DOM script.
- Removed the old `FLOATING_REFRESH_JS`, `mbaction://` navigation bridge, and `profile:floating-*` event plumbing. Browser actions now go directly from the host UI to existing Tauri commands, reducing injected code and failure points.
- Added a customizable shortcut toolbar with built-in actions: Refresh, Home, Back, Forward, Copy URL, Account Overview, Downloads, and Settings.
- Added per-button visibility and ordering, right-side/bottom placement, and a compact mode.
- Added up to 12 user-defined URL shortcut buttons, each with its own label, optional Emoji/short icon, URL, enabled state, and ordering.
- Added layout resync after toolbar preference changes so the native child WebView cannot temporarily cover the host toolbar when switching dock position or size.
- Added a clipboard fallback for Copy URL and a small host-side refresh activity animation.
- Fixed malformed markup in the preferences dialog while reorganizing settings into a dedicated Shortcut Toolbar tab.
- Bumped application version to `0.5.0`.
