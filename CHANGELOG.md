# Changelog

## 0.4.2

### Player and controls
- Refresh the TUI with a compact server/guild header and responsive layouts.
- Prioritize playback and queue control at 80×24; show visualization when space permits.
- Add queue selection, scrolling, paging, and clear focus indicators.
- Add a searchable action palette (`:` / `Ctrl+p`) and keyboard help (`?`).
- Use Space to pause/resume, `/` to add tracks, and Tab to switch focus.
- Improve Unicode text editing, cursor movement, and bracketed paste support.
- Require explicit confirmation before clearing the queue.
- Show update notices without interrupting the active screen.

### Connectivity and reliability
- Change the default webhook/API server to `https://jorik.gleckson.com`.
- Migrate saved settings using the retired default while preserving custom hosts and explicit overrides.
- Fix TUI host-override precedence, small-terminal visualization crashes, and UI locks during network reads.
- Surface command failures and WebSocket acknowledgement timeouts.
- Preserve cached queue data during transient failures and stop duplicate lyrics requests.
- Fix settings cancellation and allow quitting from connection-error screens.
- Avoid guessing playback state when the server has not supplied it.

### Update integrity
- Replace the built-in remote-shell updater with version-pinned GitHub release downloads.
- Verify asset names, URLs, size, and GitHub API SHA-256 digests before installation.
- Stage and atomically replace Linux binaries; launch only verified Windows installers.
- Reject unsafe archives, unexpected redirects, missing digests, and unverified fallbacks.
- Keep stable installations on the stable update channel.
- Publish release checksums and require matching release tags and package versions.

### Maintenance
- Add Linux/Windows CI checks, regression tests, and an 80×24 rendering snapshot.
- Redact authentication tokens from output and logs.
- Save Unix authentication files with owner-only permissions.

### Remaining work for 0.5.0
This release is not the completion of the security-hardening work:
- Tokens are still stored in JSON rather than the OS keychain.
- Authentication callback validation, per-login nonces, and cancellation need further hardening.
- Update hashes trust GitHub's HTTPS metadata; independent publisher signatures are not yet verified.
- Legacy shell installer scripts remain outside the built-in verified updater.
- Queue selection navigates loaded rows only; remove/reorder/jump actions need backend support.
