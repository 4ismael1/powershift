# PowerShift Distribution Strategy

PowerShift has two deliberately separate distribution tracks. The current
track is an NSIS installer published through GitHub Releases. The target stable
track is an MSIX package submitted to Microsoft Store.

## Current track: GitHub Releases

- Build the existing x64 NSIS installer from a clean tagged commit.
- Publish the installer and its SHA-256 checksum in the same GitHub Release.
- State clearly that the installer is unsigned and that Windows can show an
  unknown-publisher or SmartScreen warning.
- Do not add an automatic updater until update authenticity, rollback, and
  partial-failure recovery have a tested design.
- Never present the GitHub asset as Store-signed. Microsoft does not re-sign an
  EXE or MSI installer, even when that installer is listed in Microsoft Store.

This track is acceptable for the initial public releases, but its warning and
reputation cost is a known distribution limitation rather than a product bug.

## Target track: Microsoft Store MSIX

Microsoft Store automatically re-signs an MSIX/AppX package after successful
certification. A CA-trusted certificate is therefore not required for the Store
submission itself. That benefit applies to MSIX/AppX packages, not to a linked
NSIS EXE.

PowerShift cannot treat MSIX as a file-extension change. The current NSIS hook
performs machine-level setup, creates an elevated scheduled agent task, starts
the per-user tray, and owns uninstall cleanup. A Store package must reproduce
those lifecycle guarantees using package-supported behavior.

The elevated agent is the main Store-certification risk. A packaged desktop app
that depends on elevation can require the restricted `allowElevation`
capability. Microsoft subjects that capability to explicit justification and
strict approval. Store delivery is therefore a target, not an assumed outcome,
until PowerShift has received capability approval or removed that dependency.

## Store readiness gates

1. Reserve the Store identity and make the manifest publisher/name/version
   match Partner Center exactly.
2. Produce a development MSIX with the UI, agent, tray, icons, and required
   WebView/runtime dependencies. Use a self-signed certificate only for local
   testing.
3. Prototype first-run, upgrade, repair, startup, and uninstall without relying
   on the NSIS hook. Confirm that an update never strands an old scheduled task
   that points at a superseded package path.
4. Declare only the minimum full-trust/restricted capabilities and request
   `allowElevation` approval early, with a concrete explanation of why changing
   the Windows power plan needs the elevated background agent.
5. Re-run the complete agent/tray matrix from
   `docs/STABLE_RELEASE_CHECKLIST.md` on the packaged identity, including reboot,
   user SID separation, power-lease recovery, Store update, and uninstall.
6. Run Windows App Certification Kit and a private Store flight before making
   the package public.
7. Keep the GitHub NSIS and Store MSIX release pipelines separate. A successful
   NSIS release is not evidence that the MSIX lifecycle is correct.

## Signing rules

- Store MSIX/AppX: submit through Partner Center; Store signs/re-signs after
  certification.
- Local MSIX tests: use a self-signed certificate trusted only on test systems.
- MSIX sideloaded outside Store: use a production trusted signing option.
- GitHub NSIS EXE: it remains unsigned until a separate Authenticode signing
  service or certificate is introduced.
- Private keys, PFX files, control tokens, and signing credentials must never be
  committed or included in diagnostics.

## Authoritative references

- [Microsoft Store signing FAQ](https://learn.microsoft.com/en-us/windows/apps/publish/faq/get-started-with-the-microsoft-store)
- [MSIX signing overview](https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview)
- [Code-signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
- [Restricted capability declarations](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations)
- [Publishing Win32 apps through Store](https://learn.microsoft.com/en-us/windows/apps/distribute-through-store/how-to-distribute-your-win32-app-through-microsoft-store)
