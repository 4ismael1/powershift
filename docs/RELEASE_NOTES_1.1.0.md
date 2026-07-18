# PowerShift v1.1.0

PowerShift 1.1 is a backward-compatible feature and reliability release for
the 1.x line. Automated release gates are complete; the broader environment
matrix that remains unverified is documented below as an accepted limitation.

## Highlights

- Gives every associated process an explicit role. A **companion** can keep an
  already-started profile active but cannot start it alone. An **alternate
  trigger** can start the profile without its main executable.
- Adds **Take control now**, an authenticated and temporary handoff to another
  active profile. It does not rewrite saved priorities and expires as soon as
  the promoted profile becomes inactive.
- Reapplies a controlling profile's edited start plan on the next agent
  evaluation while preserving the normal priority and handoff rules.
- Preserves an external manual power-plan change instead of overwriting it when
  a delayed restore is pending.
- Corrects `path_or_name` process matching so a configured path can fall back to
  its executable name when the runtime path is unavailable or different.
- Makes the do-nothing close behavior available in the editor and improves
  recovery notices, save rollback, configuration warnings, and error handling.
- Extracts the profile editor and runtime reconciliation into focused modules,
  with rendered Vue integration coverage for the main user flow.

## Compatibility and migration

- Configuration schema version 5 is migrated automatically from supported
  earlier schemas.
- Existing associated processes default to the **companion** role, preserving
  the safe expectation that they do not cold-start a profile.
- Legacy profiles that explicitly disabled the main-process requirement retain
  their prior cold-start behavior through migration.
- Saved profile priorities and plans are not rewritten by temporary control
  handoff.
- The intended upgrade path is an in-place upgrade from `v1.0.0`; uninstalling
  the previous version first should not be necessary.

## Verification

Release verification completed on 2026-07-18:

- Version contract: `package.json`, `package-lock.json`, Cargo workspace packages,
  `tauri.conf.json`, executable metadata, and simulated tag `v1.1.0` agree.
- Frontend: 104 tests passed across 20 files.
- Rust: 230 tests passed; 1 real power-plan mutation test intentionally ignored.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Frontend production build and Tauri/NSIS production build: passed.
- npm audit: 0 vulnerabilities.
- RustSec audit: no unapproved advisory across 450 locked dependencies.
- Verified NSIS build completed successfully; authenticode status is
  `NotSigned` as documented for the GitHub distribution track.
- The release includes a SHA-256 sidecar generated from the exact uploaded
  installer. Verify a fresh GitHub download against that value.

## Installation

The final GitHub Release asset will be:

```text
PowerShift_1.1.0_x64-setup.exe
```

Publish a SHA-256 calculated from the exact uploaded asset, then verify a fresh
download against that value. A checksum from a different local build is not an
acceptable substitute.

## Known limitations and release gates

- GitHub Release installers are unsigned, so Windows SmartScreen can show an
  unknown-publisher warning.
- Installing from a standard account by entering credentials for a different
  administrator account is unsupported.
- The complete clean Windows 10/11, upgrade-from-`v1.0.0`, reboot/startup,
  protected-game, multi-session, repair, and uninstall matrix remains
  incomplete. This is an accepted release limitation tracked for the `1.1.x`
  line; automated tests cover the companion/alternate-trigger and temporary
  handoff rules.
- Microsoft Store distribution is a separate MSIX/AppX effort. The current NSIS
  installer is not Store-signed, and restricted elevation approval remains an
  open Store-readiness dependency.
