# dt-platform

Windows input, process ownership and token ACLs. Public signatures from the skeleton stay intact;
the pure `input::hit_test` and `input::point_in_right_panel` functions also run without Windows.
Other non-Windows operations return `Unsupported` (`pid_alive` returns false).

## Integration

Call `input::enable_dpi_awareness()` before querying window/cursor coordinates. It requests
per-monitor-v2 awareness and accepts a host already using that context. Load
`ClickConfig::from_env()` to honor `GLOWTUBE_CLICK_FILL` (default `0.32`, finite, in `0..=1`).
Keep each `InputWatcher` alive, then stop/drop it before player shutdown. Stop wakes and joins
the polling thread; the media worker unregisters only keys it successfully registered. A taken
key logs `media key 0x.. is taken by another app` and leaves other keys working.

Use `process::SERVICE_MUTEX`, `Local\GlowTubeDistractionService`, for the Rust supervisor. It is
distinct from Python's `Local\GlowTubeService` during migration. `single_instance` accepts a
name for isolated tests; retain its guard throughout supervision. The mutex is not acquired, so
closing the guard is enough and never releases a mutex owned by another thread.

`spawn_hidden` preserves command arguments, cwd, environment and stdio. It sets
`CREATE_NO_WINDOW | CREATE_SUSPENDED`, creates a private non-inheritable Job Object with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assigns the suspended child, then resumes its primary
thread. Assignment failure kills/reaps the suspended process before returning an error. No
user-supplied URL is interpolated into a shell command.

A monitor retains the job until root exit, terminates remaining descendants and releases the
handles; explicit tree shutdown joins it. Service termination closes all job handles in the kernel.
Keep the returned `std::process::Child` and call `kill_process_tree` for deterministic termination/reaping of
the root and descendants, including after graceful `/quit`. Dropping `Child` follows std's
behavior and does not itself stop a running process. Children created outside this API use
the Python-compatible hidden `taskkill /T /F /PID` fallback; that fallback requires a live root.

The binary's existing dt-t3 adapter needs only this method body; dt-t3 has no platform dependency:

```rust,ignore
fn restrict_to_owner(&self, path: &std::path::Path) -> dt_core::Result<()> {
    dt_platform::process::restrict_to_owner(path)
}
```

Restrict the token directory before creating a token file, then restrict the empty file before
writing its contents. The API opens the object without following a final symlink/junction,
sets its owner to the current token user SID, replaces all DACL entries with that user's full
control and protects it from inherited grants. Directories grant object/container inheritance.
Failures propagate; setup must not publish a token on failure. Only paths cross this boundary.

## Verification

Run from `rust/` with `CARGO_BUILD_JOBS=4`:

```powershell
cargo test --locked -p dt-platform -- --nocapture
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

The synthetic acceptance test recognizes **28/28** reference targets at 100%/150% scale.
`tests/generate_cards.py` renders `python/card_image.py` with the reference titles, queue and
14 click positions from `python/tools/click_test.py`, paints the CSS progress bar, resamples
at each scale and stores only the 1200x17 RGB strips as RLE. These are generated pixels, not
desktop captures. Cargo tests need neither Python nor an image decoder. Regenerate from the
repository root with `python rust/dt-platform/tests/generate_cards.py` (Pillow and NumPy needed).
This does not replace the integration owner's eventual Chromium measurement.

Seeded correlated RGB noise with exact marker speckles produces **0/400 false hits** (bar:
at most 1/400). Focused tests check the 158-of-493 threshold, per-channel tolerances, fixed
physical band, and scrub length/scan order. Windows process fixtures prove child/grandchild
shutdown, root reaping, the taskkill fallback and descendant cleanup on normal root exit. A separate
fixture exits the process holding the job to verify kernel kill-on-close without a surviving monitor.
Temporary ACL tests inspect the actual owner SID, single full-control ACE, protected DACL and
inherited grants on new children.
Tests read no real tokens, clipboard contents or media, and register no machine media keys.

The separately ignored `input::tests::live_gdi_strip_once` was manually run once on Windows:
foreground identity returned T3, and GDI returned 1200x17 RGB24 (61,200 bytes). It reads only
the cursor/window and one pixel strip, emits no pixels and drives no UI. Keep it excluded
from normal tests/CI; run it only with explicit live-read authorization.

All unsafe operations are confined to `src/win32`, under the single module-level override in
`src/lib.rs`; each block states its handle, buffer and lifetime assumptions. There are no
Windows stubs left. Playback, live click/media-key delivery, Chromium rendering and other OS
backends remain integration work outside this crate.
