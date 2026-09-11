# Download fixes

This revision fixes the embedded-browser download flow in `src-tauri/src/lib.rs`:

- Existing AI download links are scanned immediately when auto-download is enabled.
- Dynamic changes to `href`, `download`, and `target` are observed, so asynchronously-created links are handled.
- HTTP(S), blob, data, extension-based, and common download/export links are recognized for auto-download.
- `_blank` anchors are kept inside the current account WebView so WebView2 can emit the Tauri download event instead of losing the request to a denied popup.
- Programmatic `HTMLAnchorElement.click()` and `window.open()` download flows have fallbacks.
- Download settings are re-sent on every page load, avoiding a race where the initial configuration event could be missed.
- Invalid/unavailable custom download directories fall back to application data or temp directories.
- Windows-invalid/reserved filenames receive basic sanitization.

Validation performed in this environment:

- Injected JavaScript was extracted and passed `node --check`.
- Full npm/Tauri compilation could not be completed because the environment lacks Cargo and the npm cache is incomplete/offline.
