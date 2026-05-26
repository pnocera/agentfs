<!-- PNOCERA_EVERYTHING_START -->
## Everything Search

Use the local Everything CLI installed at `.codex\skills\everything\bin\es.exe` for fast file and folder searches before broad recursive scans.

Examples:

```powershell
& '.codex\skills\everything\bin\es.exe' -get-everything-version
& '.codex\skills\everything\bin\es.exe' -n 50 '*.sln'
& '.codex\skills\everything\bin\es.exe' -n 200 'vcvars64.bat'
```

Use `-path <folder>` only when passing a real folder to restrict the search. Do not use `-path` to mean "show paths"; ES normally prints full paths.

If ES reports `Error 8: Everything IPC window not found`, start the Everything desktop app for the same Windows user/elevation level as the agent shell. The Everything service alone is not enough for ES IPC searches.
<!-- PNOCERA_EVERYTHING_END -->
