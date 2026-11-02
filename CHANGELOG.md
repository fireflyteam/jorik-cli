# Changelog

## 0.5.0-dev.1 (unreleased)

### Credential and login hardening
- Store authentication data in Windows Credential Manager, macOS Keychain, or Linux Secret Service; never silently fall back to plaintext.
- Scope credentials to the canonical API server (scheme, host, port, and base path).
- Require explicit `auth migrate --confirm-server URL` consent before binding old unscoped JSON credentials to a server; verify secure storage before removing legacy files.
- Add `auth clear-legacy --confirm` for deliberate removal of old plaintext files, including when server settings are invalid or the OS store is unavailable.
- Preserve legacy files on migration failures and refuse to overwrite different scoped credentials.
- Share a nonce-bound, one-shot loopback login listener between CLI and TUI.
- Validate callback method, path, Host, Origin, body framing, parameters, and credential data; bound request size and read, write, and overall wait times.
- Support Ctrl+C cancellation in CLI login and Escape cancellation in the TUI; ignore stale or duplicate login attempts.
- Remove third-party resources from callback pages; add no-store, no-referrer, CSP and frame protection, and clear sensitive callback queries from the browser address.
- Require HTTPS for non-loopback servers and disable automatic API redirects.
- Authenticate WebSockets through a sensitive Authorization header, not URL query parameters.
- Drop the active token and cached player data when changing servers or signing out.
- Serialize blocking credential operations off the UI thread; wait for an active sign-out task when quitting.
- Redact credential debug output and zeroize owned credential buffers when dropped.

### Code exchange and backend pairing
- Exchange a short-lived one-time code using S256 PKCE; never accept bearer-query callbacks or downgrade to legacy login.
- Validate and bound credential JSON obtained directly from the configured server.
- Add a paired backend change with one-time Discord state, independent HttpOnly browser cookies, host-prefixed production cookies, bounded expiring grants, replay protection, and 30-day user tokens.
- Test the real Rust-to-backend PKCE protocol on loopback with a fake Discord identity; no production credentials or bot database are used.

### Signed builds and bootstrap
- Require GitHub/Sigstore build provenance in addition to SHA-256, pinned to the repository, workflow, exact release tag, issuer, and hosted runners.
- Fail closed without a current authenticated GitHub CLI or valid provenance; never fall back to hashes alone.
- Pin CI actions, gate release builds on platform checks, verify signed packages before publication, and publish attestation bundles.
- Replace legacy remote-shell installers with a local Python bootstrap using the same hash/provenance policy, bounded archive parsing, ELF validation, and atomic replacement without sudo.

### Transport, output, and validation
- Bound decoded HTTP responses and WebSocket messages; scrub server-controlled terminal controls/bidi overrides and redact secrets in output.
- Bound diagnostics and clamp untrusted playback values to prevent overflow or unbounded progress-bar allocation.
- Add Linux/Windows/macOS native credential-store CI, isolated installer tests, and an unavailable Linux Secret Service failure test.
- Locally validate Linux store round trips, overwrite/delete/server isolation, unavailable-store failure, backend type checking, and paired PKCE exchange.

### Release gates still requiring operation/approval
- Deploy the paired backend before enabling CLI 0.5 login; live Discord consent, denial, cancel/retry, signout and server switching need validation.
- Run hosted platform CI and test locked native-store UX; local Linux success does not prove Windows/macOS behavior.
- The first attested tag build must pass signature verification before publication. No 0.5 tag/release or backend deployment is made by this development change.
- Sigstore provenance trusts GitHub Actions/OIDC and is not an independent offline publisher key or Windows Authenticode signature.

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
