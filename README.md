<div align="center">

<img height="256" alt="jorik" src="https://github.com/fireflyteam/jorik-cli/blob/main/installer/assets/logo.png?raw=true"></img>

under development  
powerful "remote" for Jorik Systems based Snail bot (also WIP)

</div>

## Releases

See [CHANGELOG.md](CHANGELOG.md) for 0.4.2 changes and the security-hardening work planned for 0.5.0.

## Interactive player

```sh
cargo run -- tui
# Or, with an installed binary:
jorik tui
```

Log in with Enter when prompted, or run `jorik auth login` first.
The default API/webhook server is `https://jorik.gleckson.com`.
Override it with `--base-url`, `JORIK_BASE_URL`, or TUI settings.
Saved settings pointing to the retired `jorik.xserv.pp.ua` default are migrated
on load; custom hosts and explicit overrides remain unchanged.

The header shows the server, guild, and WebSocket status; REST remains the fallback.

### Controls

| Key | Action |
| --- | --- |
| Space / p | Pause or resume |
| / / Enter | Add a track or URL |
| Tab / Shift+Tab | Switch playback and queue focus |
| ↑↓ / j k | Select and scroll queue rows |
| PageUp / PageDown, Home / End | Page through the queue, or jump to its ends |
| : / Ctrl+p | Search all actions |
| ? | Keyboard help |
| s / w / l | Skip / stop / cycle loop mode |
| c | Clear queue, with explicit confirmation |
| v | Toggle visualization |
| 1 / 2 / 3 / 4 | Player / lyrics / settings / diagnostics |
| u | Review an available update |
| Esc | Cancel editing, close a dialog, or dismiss a notification |
| q / Ctrl+c | Quit; q remains text in input fields |

Track input supports cursor movement, Home/End, Delete/Backspace, bracketed paste,
and Ctrl+u to clear. Settings use Enter to save and Escape to discard text edits.

The player adapts to the terminal: an 80×24 window prioritizes playback and queue;
wide, tall windows can also show visualization. Zen remains a presentation mode;
switching focus to the queue makes queue navigation visible again. Visualization
visibility is a session preference; theme and layout settings remain persistent.

Queue selection currently navigates the loaded rows; it does not play, remove,
or reorder tracks. Updates appear as dismissible notices rather than interrupting
the active screen. Tab now changes focus; use `:` for the former menu.

## Verified updates

Updates require confirmation. The updater re-fetches metadata for the exact
selected GitHub release tag and accepts only the matching platform asset from
`fireflyteam/jorik-cli`. It checks the declared size and GitHub API SHA-256 digest
before extracting or launching anything. Missing digests, mismatched hashes,
unexpected URLs/redirects, oversized downloads, and malformed archives abort the
update; there is no remote-shell-script or unverified fallback.

- **Linux x86_64 GNU:** a single regular `jorik` ELF binary is staged beside the
  installed executable, synced, and atomically renamed into place. Validation or
  staging failures leave the installed file untouched. The installation directory
  must be writable; the updater never invokes sudo. Permission failures show a
  version-pinned manual release URL. Restart Jorik after installation.
- **Windows x86_64 MSVC:** the version-matching installer is verified in a private
  temporary directory, then launched directly for the existing installation
  directory. Jorik exits to let installation finish. This requires the installed
  `jorik.exe`, not a development `jorik-cli.exe`. The verified installer directory
  is printed and retained for the installer; it can be removed after it finishes.
- Other targets require manual installation. Stable versions do not automatically
  offer prereleases; prerelease versions may advance to newer prereleases or stable
  releases. Same-version reinstalls and downgrades are rejected.

**Trust boundary:** hashes come from the [GitHub release asset API](https://docs.github.com/en/rest/releases/assets#get-a-release-asset)
over HTTPS, not an independently
trusted signing key. This verifies downloaded bytes against GitHub's release
metadata; it does not protect against a compromised publisher account/repository
or compromised build. Release CI also publishes an unsigned `SHA256SUMS` file for
manual checking and rejects tags that disagree with `Cargo.toml`.

## Checks

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

Optional live verification downloads a published release into temporary files,
checks its hash and archive, then deletes it. It never runs or installs the binary:

```sh
cargo test --locked updater::tests::published_release_verifies_without_installing -- --ignored
```
