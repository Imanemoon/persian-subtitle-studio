# Phase 3: SubSource CLI Discovery Report

**Date**: 2026-11-08  
**Tool**: sub-rs.exe v1.x (SubSource API client)  
**Location**: `%LOCALAPPDATA%\Programs\SubSource\sub-rs.exe`

## Actual CLI Contract

### Command Structure
```bash
sub-rs.exe [OPTIONS]
```

**No subcommands exist.** The tool operates in **batch directory mode only**.

### Key Options
```
-d, --directory <DIRECTORY>    Target directory (required for batch)
    --top <TOP>                Max results per video [default: 5]
    --api-key <API_KEY>        SubSource API key (auth)
-l, --lang <LANG>              Language code [default: fa]
    --skip-existing            Skip videos with existing subtitles
    --no-lang-suffix           Save as <video>.srt instead of <video>.fa.srt
    --clean-ads                Remove ad cues from Farsi subtitles
    --dry-run                  Simulate without downloading
    --no-recursive             Don't scan subdirectories
    --proxy <PROXY>            HTTP proxy
    --connect <CONNECT>        Connection method [direct|system|manual|cloudflare]
    --worker <WORKER>          Cloudflare Worker URL (with --connect cloudflare)
    --gui                      GUI mode (not relevant for CLI integration)
```

### Operational Mode

**Batch Processing Flow**:
1. CLI scans directory for video files
2. For each video, searches SubSource API by filename
3. Downloads best match (or top N if configured)
4. Downloads arrive as `.zip` archives in `<video_dir>/sub/` subdirectory
5. Archives contain subtitles for one or multiple episodes
6. ZIP naming: `<video_basename>_sub_<id>.zip`

**Critical Behavior**:
- No per-file search/download commands
- No JSON output mode discovered
- No way to preview search results without downloading
- Archives may contain multi-episode packs (e.g., S01E01-12.srt)
- Tool respects `--skip-existing` flag (checks for `<video>.fa.srt`)
- Exit code 0 = success, non-zero = errors (downloads may be partial)

### Authentication
- Requires `--api-key` for SubSource API access
- Must be stored securely (not in git, not in logs)
- Key should come from user settings

## Archive Format Handling

### ZIP Structure Discovered from PowerShell Script

1. **Single Episode Archives**:
   ```
   Show_S01E01_sub_<id>.zip
     └─ Show.S01E01.1080p.WEB-DL.fa.srt
   ```

2. **Multi-Episode Archives**:
   ```
   Show_S01E01_sub_<id>.zip
     └─ Show.S01E01.1080p.WEB-DL.fa.srt
     └─ Show.S01E02.1080p.WEB-DL.fa.srt
     └─ Show.S01E03.1080p.WEB-DL.fa.srt
     ...
   ```

3. **RAR with ZIP Extension**:
   - SubSource may download RAR files with `.zip` extension
   - 7-Zip handles format detection automatically
   - Must not rely on extension alone

### Extraction with 7-Zip

**Executable**: `C:\Program Files\7-Zip\7z.exe`  
**Command**: `7z.exe x -y "-o<output_dir>" <archive_path>`

**Exit Codes**:
- 0 = success
- 1 = warning (non-fatal, e.g., some files extracted)
- 2+ = error

**Format Detection**: 7-Zip inspects file magic bytes, not extension.

## Episode Matching Logic from PowerShell

### Pattern Recognition
```regex
(?i)S(?<season>\d{1,2})E(?<episode>\d{1,2})
```

### Episode Matching in Archives
```regex
(?i)(?:^|[^A-Z0-9])S0?{season}E0?{episode}(?:[^0-9]|$)
```

**Critical Rules**:
- `S01E10` must NOT match `S01E01` or `S01E1`
- Zero-padding is normalized: `S1E1` == `S01E01`
- Word boundaries prevent false matches
- Multi-episode ranges like `E01-12` are detected but not matched for specific episodes

### Quality Preference (from PowerShell)
1. 1080p (highest priority)
2. 720p (medium)
3. Others (lowest)
4. File size (descending, as tiebreaker)

## Hard Link Staging Pattern (from PowerShell)

### Purpose
Create temporary directory tree with hard links to original videos:
```
<root>/_SubSource_Batch_<guid>/
  └─ Season 01/
      ├─ Show.S01E01.mkv  (hard link to original)
      ├─ Show.S01E01.fa.srt  (hard link if exists, to skip download)
      ...
```

### Benefits
- SubSource CLI processes entire directory structure
- No video file duplication (hard links share inode)
- Existing subtitles prevent re-download via `--skip-existing`
- All downloads land in known staging area
- Cleanup doesn't affect originals

### Constraints
- Only works on same filesystem/drive
- NTFS supports hard links for files (not directories)
- Hard link changes DO affect original if file is modified in-place
- Our use case: read-only video files, write-only subtitle files (safe)

## Security Considerations

1. **API Key Storage**: The API Key is stored exclusively in the **Windows Credential Manager**
   (`PersianSubtitleStudio` / `SubSourceApiKey`). It is **never** written to `settings.json` or
   any config file on disk.

2. **Command-Line Exposure**: The SubSource CLI (`sub-rs.exe`) accepts the API key only via the
   `--api-key` argument — there is no alternative mechanism (e.g., environment variable or config
   file). This means the credential **is passed on the command line** during process execution and
   **may be visible** to process inspectors (e.g., Task Manager, Process Explorer, `ps` equivalents)
   with sufficient system privileges. Redaction applies only to our own logs, stdout, stderr, and
   handled error messages; it does **not** mitigate this command-line visibility limitation.

3. **Path Traversal**: Validate archive contents before extraction.
4. **Staging Isolation**: Staging directory must be inside project root or user-approved location.
5. **Archive Validation**: Check extracted files are subtitle formats only.

## Implementation Requirements from Discovery

### Backend Module Design
```
src-tauri/src/subsource/
  ├─ mod.rs           (public API)
  ├─ cli.rs           (sub-rs.exe execution)
  ├─ staging.rs       (staging directory + hard links)
  ├─ archive.rs       (7-Zip extraction + matching)
  ├─ plan.rs          (download plan generation)
  └─ types.rs         (DTOs for frontend)
```

### Commands (Tauri)
```
src-tauri/src/commands/subsource.rs
  - create_download_plan(videos: Vec<VideoInfo>) -> Plan
  - execute_download_plan(plan_id: String) -> Progress
  - cancel_download(operation_id: String)
  - get_download_status(operation_id: String) -> Status
```

### Settings Integration

The `SubSourceSettings` struct (serialized under the `subsource` key in `settings.json`) contains
only non-sensitive fields:

```json
{
  "subsource": {
    "connectionMode": "direct",
    "proxy": null,
    "worker": null,
    "cleanAds": true,
    "skipExisting": true
  }
}
```

> **Note**: The API Key is **not** stored in `settings.json`. It lives exclusively in the
> **Windows Credential Manager** (service `PersianSubtitleStudio`, account `SubSourceApiKey`).
> The UI reads it at runtime via `retrieve_api_key()` and passes it to the CLI via `--api-key`.

### Workflow Stages

1. **Pre-Check**: Skip videos with `.fa.srt` or `.fa.synced.srt`
2. **Staging Setup**: Create temp dir, hard-link videos + existing subs
3. **Download**: Execute `sub-rs.exe` on staging directory
4. **Extraction**: Process each ZIP with 7-Zip
5. **Matching**: Find correct subtitle for each episode
6. **Validation**: Run Phase 2 subtitle validator
7. **Placement**: Move validated `.fa.srt` next to original video
8. **Cleanup**: Remove staging directory

### Error Scenarios to Handle

- SubSource CLI not found
- API key missing or invalid
- Network failure during download
- Partial downloads (exit code != 0)
- Archive extraction failure
- No matching subtitle in archive
- Multiple ambiguous matches
- Non-Persian content in subtitle
- Validation failure
- Disk space exhausted
- Hard link creation failure (cross-drive)

## Open Questions for User

1. **API Key Source**: How should users configure SubSource API key?
   - Settings UI field?
   - Environment variable?
   - Config file?

2. **Connection Mode**: Default to "direct" or detect proxy from system?

3. **Download Location**: Stage in project temp dir or system temp?
   - Project temp: predictable, easy cleanup
   - System temp: respects user's temp drive preference

4. **Conflict Resolution**: When archive has multiple matching subtitles?
   - Take highest quality automatically?
   - Prompt user to choose?

5. **Archive Retention**: Keep or delete downloaded ZIPs after extraction?
   - Keep in `_SubSource_ZIP_Failures` for multi-episode recovery (per PowerShell pattern)
   - Delete to save space
   - User preference

## Next Steps

1. Read existing codebase architecture
2. Design subsource module matching project conventions
3. Implement staging + CLI execution
4. Implement 7-Zip extraction + matching
5. Add Tauri commands
6. Connect to Processing UI
7. Write comprehensive tests (mocked + live)
