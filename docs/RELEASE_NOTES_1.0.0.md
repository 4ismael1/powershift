# PowerShift v1.0.0

PowerShift 1.0 is the first stable code line for automatic Windows power-plan
switching. It combines a temporary Tauri configuration interface with a small
native Rust agent and tray that remain after the UI closes.

## Highlights

- Tracks configured processes by PID and creation time, with Windows process
  exit waits for direct close detection.
- Uses independently supervised WMI start and stop channels with bounded
  backoff and degraded reconciliation instead of continuous global polling.
- Resolves simultaneous profiles deterministically: the highest priority active
  profile controls, then the next eligible profile takes over when it exits.
- Keeps automation alive after the WebView UI closes; only the Rust agent and
  notification-area companion remain resident.
- Restores the previous power plan after configured delays and preserves a
  crash-safe power-control lease across unexpected exits.
- Uses session-scoped IPC, authenticated mutable commands, per-user runtime ACLs,
  atomic configuration recovery, bounded logs, and bounded icon caches.
- Adds complete task installation/repair validation, clean upgrade shutdown,
  privacy-bounded support diagnostics, and stable publisher/version metadata.
- Refines profile naming, icons, notifications, drawers, keyboard accessibility,
  long-process layouts, and agent health reporting.

## Measured runtime

Five-minute sample with the UI closed and 18 Chrome instances tracked:

- Agent plus tray CPU time: **0.015625 s**.
- Agent plus tray average working set: **16.64 MiB**.
- Agent plus tray average private memory: **5.18 MiB**.
- `agent-state.json`: **no writes while functional state remained unchanged**.

A separate installed idle sample used 12.29 MiB combined working set and
4.44 MiB private memory. These are measurements from one Windows system, not
universal guarantees.

## Verification

- Rust: 219 tests passed; 1 real power-plan mutation test intentionally ignored.
- Frontend: 98 tests passed.
- Formatting and Clippy with warnings denied: passed.
- npm audit: 0 vulnerabilities.
- RustSec audit: no unapproved advisory.
- Tauri/NSIS production build: passed.

## Installation

Download `PowerShift_1.0.0_x64-setup.exe` from this release.

**SHA-256:** `25E9E1B9849A78BAA319C188D9D4DE374939896536CC12B157660D1747F8719F`

PowerShift targets Windows 10/11 x64 and an administrator Windows account. The
installer registers one elevated, SID-scoped agent task and a per-user tray.
Existing valid preview configuration is preserved during upgrade.

## Known limitations

- PowerShift is distributed unsigned. Windows SmartScreen may display an
  unknown-publisher warning; verify the SHA-256 above before running it.
- Installing from a standard account by supplying credentials for a different
  administrator account is not a supported scenario.
- Broader clean-VM, protected-game, launcher, and multi-session validation
  remains planned for the 1.0.x line.
