# PowerShift Support Diagnostics

PowerShift includes a local diagnostic collector for reproducible bug reports.
Run it from the repository root with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\collect-diagnostics.ps1
```

The script creates a timestamped ZIP under `PowerShift-Diagnostics` on the
desktop. It records:

- Windows version and current power plan.
- Installed PowerShift binary versions, SHA-256 hashes, and signature status.
- Agent/tray process resource counters.
- The current user's scheduled task status.
- Published agent state and bounded event history, when available.

It deliberately excludes `config.json`, `agent-control.token`, and
`power-control-lease.json`. Agent state and events may still contain profile or
executable names, so review the archive before sharing it publicly.
