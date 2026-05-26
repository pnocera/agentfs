---
name: everything
description: Local Windows file and folder search using the bundled Everything es.exe executable. Use when Codex or Claude needs to find local files by filename, extension, wildcard, Boolean expression, path, regex, files-only/folders-only scope, or quick result count; prefer this over broad recursive filesystem scans when the search is not confined to a known small directory.
---

# Everything

Use the bundled executable directly. Resolve it relative to this skill file:

```powershell
& "<this-skill-dir>\bin\es.exe" -n 50 "*.pdf"
```

For broad searches, cap output with `-n <count>` unless the user explicitly asks for exhaustive results. Use `-get-result-count` when only a count is needed.

Sanity-check IPC before diagnosing query syntax:

```powershell
& "<this-skill-dir>\bin\es.exe" -get-everything-version
```

If this returns `Error 8: Everything IPC window not found`, `es.exe` cannot see the Everything desktop IPC server. Make sure the Everything desktop app is running for the same Windows user and elevation level as the agent shell; the Everything service alone is not enough for ES IPC searches.

Useful examples:

```powershell
& "<this-skill-dir>\bin\es.exe" -n 50 "*.docx"
& "<this-skill-dir>\bin\es.exe" -n 50 "*.ps1|*.psm1"
& "<this-skill-dir>\bin\es.exe" -n 50 "invoice !backup"
& "<this-skill-dir>\bin\es.exe" -n 50 /a-d "*.log"
& "<this-skill-dir>\bin\es.exe" -n 50 /ad "Tools"
& "<this-skill-dir>\bin\es.exe" -n 50 -r ".*\.sln$"
& "<this-skill-dir>\bin\es.exe" -get-result-count "*.md"
```

Use `-path <folder>` only to restrict the search to a specific indexed folder:

```powershell
& "<this-skill-dir>\bin\es.exe" -path "C:\Users\Pierre\Downloads" -n 50 "*.zip"
```

Do not use `-path` to mean "show full paths"; `es.exe` normally prints full paths, and `-full-path-and-name` is the display-column option when an explicit column is needed.

CLI syntax supports wildcards (`*`, `?`), extension searches (`*.pdf`), Boolean OR with `|`, NOT with `!`, regex with `-r`, files-only with `/a-d`, and folders-only with `/ad`.

Do not rely on Everything GUI-only date filters such as `modified:today`, `modified:7days`, or `dm:today` in CLI workflows. For date filtering, use `es.exe` to get a bounded candidate list, then filter those paths with PowerShell or another filesystem API.
