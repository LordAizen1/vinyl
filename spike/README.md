# vinyl SMTC spike

This throwaway Phase 0 program reports every Windows System Media Transport Controls
session once per second. It reads metadata, playback status, timeline values, and the
thumbnail byte count. The header reports the current app ID and title, which identifies
Windows' exact choice even when one app owns several sessions.

Run it from PowerShell:

```powershell
cd spike
cargo run
```

Press `Ctrl+C` to stop. Exercise the sources listed in `FINDINGS.md`, then fill in the
table. The spike intentionally reads thumbnail bytes on every pass so failures and
inconsistent providers are visible during the test.

For a single snapshot instead of the continuous test loop, run:

```powershell
cargo run -- --once
```
