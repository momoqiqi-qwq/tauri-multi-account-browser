# V9 changes

- Any `https://files.chat01.ai/python-generations/...` download now returns the account webview to `https://chat01.ai/` after the download finishes; these transient download URLs are never persisted as the account's last page.
- Added a bottom-right Home button beside Refresh inside every account webview. Home always navigates to `https://chat01.ai/`.
- Added a persistent download ledger separate from visible download history. A successfully downloaded file is blocked from downloading again by source URL or filename, even after its history record or local file is deleted. Existing successful history is migrated into the ledger.
- Redesigned both sidebars with clearer WinUI-inspired borders, layered cards, active accents, shadows, and brighter hierarchy.
- Renamed the all-account view to “全部账号历史” and added a top category strip for quick filtering.
