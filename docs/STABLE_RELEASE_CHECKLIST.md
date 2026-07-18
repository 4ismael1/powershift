# PowerShift Stable Release Checklist

This checklist is the release contract for PowerShift 1.x. Automated checks
reduce risk, but they do not replace validation on clean Windows installations.
Any release with an incomplete gate must document the exception, owner, and
follow-up explicitly instead of presenting the gate as completed.

## 1. Release Identity

- [ ] `package.json`, Cargo workspace packages, `tauri.conf.json`, and the tag
      expose the same `MAJOR.MINOR.PATCH` version.
- [ ] The release commit is reviewed, has a clean working tree, and is reachable
      from `main`.
- [ ] User-facing changes and known limitations are documented in release notes.
- [ ] The stable tag has no preview, alpha, beta, or release-candidate suffix.

## 2. Automated Quality Gates

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test --workspace` passes.
- [ ] `npm test` passes.
- [ ] `npm run build:frontend` passes.
- [ ] `npm audit --audit-level=high` reports no high or critical finding.
- [ ] `cargo audit -D warnings` reports no unapproved vulnerability or informational advisory.
- [ ] CI uses the repository-pinned Node and Rust toolchains.
- [ ] The Windows CI workflow builds the NSIS installer from a clean checkout.

## 3. Initial GitHub Release Integrity

- [ ] The release is explicitly identified as unsigned; no verified-publisher
      claim appears in documentation or release metadata.
- [ ] The release includes a SHA-256 checksum for the exact installer asset.
- [ ] A fresh download from GitHub matches the published SHA-256 checksum.
- [ ] Windows UAC displays the expected product name and PowerShift icon; an
      unknown-publisher warning is expected.
- [ ] No token, log, local config, generated binary, certificate, or private key
      is tracked by Git.

PowerShift releases are published manually through GitHub Releases. An in-app
updater is deferred until a secure update-verification and rollback strategy has
been designed and tested.

The target Microsoft Store MSIX track is a separate lifecycle. Follow
`docs/DISTRIBUTION.md`; Store signing does not apply to the GitHub NSIS EXE.

## 4. Clean Installation Matrix

Run each required case on clean virtual machines, not only the development PC.

- [ ] Windows 11 x64, current stable channel, administrator user.
- [ ] Windows 10 x64, latest supported release, administrator user.
- [ ] A standard-user launch that supplies different administrator credentials
      is either rejected with a clear requirement or fully validated against
      the intended interactive account; it must never install silently for the
      wrong SID.
- [ ] Installation with WebView2 already present.
- [ ] Installation where the WebView2 bootstrapper must run.
- [ ] Installer is launched from a standard download location and SmartScreen,
      UAC, and antivirus behavior are recorded.
- [ ] Installer creates exactly one SID-scoped elevated scheduled task.
- [ ] Agent and tray start without a visible PowerShell or console window.
- [ ] Tray opens the UI, exposes the current plan in its tooltip, and exits all
      PowerShift processes when the user chooses Exit.
- [ ] First launch does not flash white, resize unexpectedly, or show clipped
      controls at 958 x 598 and the minimum supported window size.

## 5. Reboot and Startup

- [ ] Default installation survives sign-out, reboot, shutdown, and cold boot.
- [ ] The elevated agent starts once, reaches `running`, and both WMI channels
      reach `running` without user intervention.
- [ ] The tray starts once and opens a fresh UI on demand.
- [ ] Disabling **Start with Windows** disables future startup without killing a
      currently active profile or deleting the task.
- [ ] Re-enabling startup restores the task trigger and tray autostart.
- [ ] **Start in background** controls UI visibility without changing agent
      availability.

## 6. Detection and Power-Plan Behavior

- [ ] Chrome: multi-process start, child exits, full close, and rapid reopen.
- [ ] Fortnite: launcher transition, anti-cheat process, game exit, and delayed
      helper shutdown.
- [ ] GTA V: launcher/game transition and full close.
- [ ] At least one Steam game and one standalone executable.
- [ ] PID reuse and rapid start/stop do not leave a stale active profile.
- [ ] Multiple instances of one executable remain active until the last tracked
      instance exits.
- [ ] An unrelated process start or stop does not trigger a global snapshot.
- [ ] A higher-priority active profile takes control immediately.
- [ ] A lower-priority profile remains active but cannot override the winner.
- [ ] Equal-priority profiles keep the current winner to avoid plan oscillation.
- [ ] Closing the winner transfers control to the next active profile.
- [ ] A companion process cannot cold-start a profile, keeps a previously
      started session active, and releases it after both main and companions exit.
- [ ] An alternate trigger can start its profile without the main executable.
- [ ] **Take control now** overrides priority only while the promoted profile
      remains active, then automatically returns to normal conflict resolution.
- [ ] Editing the controlling profile's start plan reapplies the new configured
      plan on the next agent evaluation.
- [ ] Previous-plan and specific-plan restoration both honor their configured
      delay, including zero delay.
- [ ] Pausing automation stops plan changes while the agent remains healthy.

PowerShift must remain observation-only toward games: no injection, hooks,
process writes, memory reads, or anti-cheat integration.

## 7. Failure and Recovery

- [ ] Killing the agent lets Task Scheduler restart it without losing the power
      control lease.
- [ ] A crash or forced shutdown restores the captured pre-PowerShift plan on the
      next safe startup or uninstall.
- [ ] WMI start and stop watcher failures enter bounded degraded reconciliation,
      retry with backoff, and recover without a restart storm.
- [ ] Corrupt primary config recovers from a valid backup.
- [ ] Corrupt primary and backup are preserved for diagnosis, defaults are
      created safely, and the UI shows one recovery notice.
- [ ] A config from a future schema version is never overwritten.
- [ ] Invalid, oversized, or excessive profile/process collections are rejected.
- [ ] IPC rejects malformed, oversized, cross-user, and invalid-token mutation
      requests without stopping the agent.

## 8. Resource and Stability Budgets

Measure release binaries for at least 30 minutes with the UI closed and again
through a repeated start/stop workload.

- [ ] Idle combined CPU remains below 0.1% of one logical processor on the test
      system, with no periodic spike pattern.
- [ ] Agent plus tray private memory remains below 20 MiB in steady state.
- [ ] Agent plus tray working set remains below 35 MiB in steady state.
- [ ] `agent-state.json` is written only when functional state changes.
- [ ] Event logs remain bounded and rotate at the configured limit.
- [ ] Handles and threads return to baseline after 500 tracked process exits;
      sustained unexplained growth fails the release.
- [ ] Closing the UI terminates the Tauri host and every WebView2 child while the
      Rust agent and tray continue normally.

## 9. Upgrade, Repair, and Uninstall

- [ ] Upgrade from the latest preview preserves valid profiles and preferences.
- [ ] Opening the UI or upgrading does not restart a healthy agent or lose the
      captured previous plan.
- [ ] Repair replaces missing binaries/task registration and starts the agent
      without creating duplicate tasks or tray instances.
- [ ] Uninstall first releases PowerShift power control, then stops the tray and
      agent, removes scheduled tasks and autostart, and deletes owned runtime
      data.
- [ ] Uninstall leaves the user's active power plan in a valid expected state.
- [ ] A post-uninstall scan finds no PowerShift process, scheduled task, Run key,
      Program Files directory, ProgramData runtime, or local WebView data.

## 10. Product and Support Readiness

- [ ] Spanish UI text is consistent, concise, and free of internal GUIDs or raw
      HRESULT messages in normal notifications.
- [ ] Every icon-only control has an accessible name and keyboard focus state.
- [ ] Drawers trap focus, close with Escape, restore focus, and never overlap a
      confirmation dialog.
- [ ] Empty, loading, success, degraded, and failure states are understandable.
- [ ] README installation instructions, architecture, measurements, license, and
      recovery guidance match the shipped version.
- [ ] The collector in `scripts/collect-diagnostics.ps1` is exercised on the
      signed build and its archive contains no config, IPC token, power lease,
      or unrelated personal data.

## 11. Microsoft Store MSIX Readiness

- [ ] Store identity, publisher, package name, and four-part package version are
      reserved in Partner Center and match the manifest.
- [ ] A development MSIX packages the UI, elevated agent, tray, and runtime
      resources without depending on NSIS hooks.
- [ ] Microsoft has approved the restricted elevation capability, or the agent
      architecture no longer requires it.
- [ ] Packaged first launch, startup, upgrade, repair, power-lease recovery, and
      uninstall pass on clean Windows systems.
- [ ] Windows App Certification Kit passes and a private Store flight installs,
      updates, launches, and uninstalls successfully.
- [ ] The submitted artifact is MSIX/AppX; documentation does not imply that
      Store re-signs a linked EXE/MSI installer.

## Release Decision

Record the Windows versions, hardware, installer SHA-256, test date, reviewer,
and any accepted limitation in the release notes. Stable approval requires all
mandatory boxes above or an explicitly documented exception with owner and
follow-up version.

### v1.0.0 accepted exceptions

- Owner: `4ismael1`.
- PowerShift is intentionally distributed unsigned. SmartScreen may warn; every
  release must publish and verify a SHA-256 checksum for the installer.
- Automated tests, local install/reboot, Chrome activation/restore, IPC, WMI,
  task elevation, ACL, uninstall, and resource checks were exercised on the
  development system. The complete clean-VM and protected-game matrix remains
  follow-up validation for `1.0.x`.

### v1.1.0 release decision

- Owner: `4ismael1`.
- The version target is `1.1.0`: associated-process roles and temporary manual
  control handoff are backward-compatible features, so SemVer requires a minor
  increment from `1.0.0`.
- Automated checks, dependency audits, version verification, and the local NSIS
  build passed and are recorded in `docs/RELEASE_NOTES_1.1.0.md`.
- Owner `4ismael1` explicitly approved publication with the complete clean
  Windows 10/11, upgrade, reboot/startup, protected-game, multi-session, repair,
  and uninstall matrix incomplete. This is an accepted limitation with
  follow-up assigned to the `1.1.x` line, not a claim that those gates passed.
- The GitHub NSIS release remains unsigned. The separate Microsoft Store MSIX
  lifecycle is not a gate for the GitHub release and must not be represented as
  completed by this candidate.
