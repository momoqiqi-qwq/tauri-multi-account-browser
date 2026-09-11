# V6 changes

- Added configurable active-tab auto refresh (0 disables, 1-86400 seconds).
- Added refresh position restoration using sessionStorage (scroll X/Y + URL hash), used by both automatic and manual reload when enabled.
- Auto-refresh timer resets when switching profiles and pauses while host UI overlays suppress the webview.
- Preserved and integrated the 11 theme presets already present in v5: Classic, Claude Light/Dark, Midnight Blue, Graphite, Glass Blue, Paper Cream, Amethyst, Win11, Sunset Orange/Purple, Mint Light.
- Special handling for the requested files.chat01.ai ZIP: it is not persisted as profile last_url, and after the download finishes the webview navigates back to https://chat01.ai/.
- Kept download/history and existing UX optimizations intact.

## Verification note
The source was statically inspected after modification. Full frontend/Rust compile checks could not be completed in this sandbox because npm dependency installation timed out and the Rust cargo toolchain is unavailable.
