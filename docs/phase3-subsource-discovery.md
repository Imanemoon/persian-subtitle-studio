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
    --gui                      GUI mode (not relevant for CLI integration)
```

> **Note**: `--proxy`, `--connect`, and `--worker` exist in the CLI but are **not used** by this
> integration. SubSource uses the Windows network connection directly. Users can enable a VPN
> externally if they need to route traffic through a different network path.

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
- Secure storage via Windows Credential Manager is implemented in `subsource::secrets` (`store_api_key`, `retrieve_api_key`, `delete_api_key`)
- In Phase 3 PR 2, `retrieve_api_key()` is invoked before creating any staging files or reserving operation slots. If the key is absent or empty, the operation aborts immediately with an explanatory error message, ensuring zero filesystem modifications.
- The retrieved key is passed into `BatchOptions.api_key` for CLI execution in memory; it is never written to `settings.json` or any disk file.
- Direct UI integration for entering the key into Credential Manager via the frontend settings page remains slated for a future phase.

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

## Hard Link Staging Pattern

### Implementation in Phase 3 PR 2
Create temporary directory tree per volume with hard links to original videos:
```
<volume_root>/_subsource_batch_<uid>/
  └─ <relative_dir>/
      ├─ Show.S01E01.mkv  (hard link to original)
      └─ sub/             (created by SubSource containing downloaded ZIPs)
          └─ Show.S01E01_sub_1.zip
```

### Operational Rules & Platform Limitations
1. **Volume Boundary Limitation**: On Windows (NTFS) and Unix filesystems, hard links can only exist within the same filesystem volume. Videos spanning multiple volumes (e.g. `C:\` and `D:\`) are partitioned via `group_by_volume()`. Each volume gets an independent staging root on that same drive.
2. **Zero-Copy Guarantee**: Large video files are strictly linked via `std::fs::hard_link`. The application **never** falls back to copying video files. If hard linking fails, the entire staging structure for that group is cleanly rolled back (`rollback()`), and original files are left untouched.
3. **Scanner Exclusion**: Staging folders use the prefix `_subsource_batch_<uid>`. The scanner module explicitly ignores directories starting with `_subsource_`, ensuring staged hard links are never indexed as new library entries.
4. **Ownership Tracking & Selective Cleanup**: The `StagingSession` explicitly tracks every hard link and directory created by the application:
   - On completion, error, or cancellation: `cleanup_keep_output()` deletes only the application's temporary video hard links and empty directories.
   - The downloaded `.zip` archives and the staging root are **preserved** for the subsequent Archive Extraction phase (PR 3).
   - If staging construction fails before SubSource executes, `rollback()` removes all created links and empty directories including the staging root.
5. **No Overwrites**: Existing final subtitles (such as `.fa.srt` or `.fa.synced.srt`) are checked both during plan generation and again at execution time; affected videos are skipped so existing subtitles are never overwritten.

## Security Considerations

1. **API Key Storage**: The API Key is stored exclusively in the **Windows Credential Manager**
   (`PersianSubtitleStudio` / `SubSourceApiKey`). It is **never** written to `settings.json` or
   any config file on disk. `retrieve_api_key()` checks the key before any staging or filesystem operations.

2. **Command-Line Exposure**: The SubSource CLI (`sub-rs.exe`) accepts the API key only via the
   `--api-key` argument — there is no alternative mechanism (e.g., environment variable or config
   file). This means the credential **is passed on the command line** during process execution and
   **may be visible** to process inspectors (e.g., Task Manager, Process Explorer, `ps` equivalents)
   with sufficient system privileges. Redaction applies only to our own logs, stdout, stderr, and
   handled error messages; it does **not** mitigate this command-line visibility limitation.

3. **No Proxy Support**: Proxy configuration has been removed from this integration. SubSource
   uses the Windows network connection directly. Users who need to route traffic through a proxy
   or VPN should configure that at the OS or VPN client level before running the application.

4. **Path Traversal & Unicode**: Hard-link paths and directory structures support Persian filenames, Unicode paths, and paths with spaces. Paths must be absolute.

## Implementation Requirements from Discovery

### Backend Module Design
```
src-tauri/src/subsource/
  ├─ mod.rs           (public API, re-exports)
  ├─ cli.rs           (sub-rs.exe Command wrapper, cancellation token)
  ├─ staging.rs       (volume grouping, hard-link staging, selective cleanup)
  ├─ plan.rs          (download plan generation, sidecar detection, validation)
  ├─ secrets.rs       (Windows Credential Manager integration)
  ├─ types.rs         (BatchOptions, BatchResult, error classification)
  └─ error.rs         (SubSourceError taxonomy)
```

### Commands (Tauri)
```
src-tauri/src/commands/subsource.rs
  - plan_subsource_download(paths: Vec<String>) -> Result<DownloadPlan, String>
  - execute_subsource_download(items: Vec<DownloadPlanItem>) -> Result<u64, String>
  - is_download_active() -> bool
```

Cancellation is integrated with the existing `cancel_subtitle_ops` command via `OpsRegistry::cancel_all()`.

### Settings Integration

The `SubSourceSettings` struct (serialized under the `subsource` key in `settings.json`) contains
only non-sensitive fields. Proxy and connection-mode settings have been removed — SubSource uses
the Windows network connection directly.

```json
{
  "subsource": {
    "cleanAds": true,
    "skipExisting": true
  }
}
```

> **Note**: The API Key is **not** stored in `settings.json`. It is managed via the
> **Windows Credential Manager** (service `PersianSubtitleStudio`, account `SubSourceApiKey`)
> through helper functions in `subsource::secrets`.
>
> In Phase 3 PR 2, `execute_subsource_download` retrieves the API key via `retrieve_api_key()`
> before initiating staging or batch operations. Adding a dedicated Settings UI input for
> credential management is scheduled for a future UI integration phase.

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

1. **API Key UI**: The API Key is currently stored in the Windows Credential Manager. A Settings
   UI field for entering/updating it is planned for a future phase. Until then, the key must be
   stored via an external tool or the system Credential Manager UI.

2. **Download Location**: Stage in project temp dir or system temp?
   - Project temp: predictable, easy cleanup
   - System temp: respects user's temp drive preference

3. **Conflict Resolution**: When archive has multiple matching subtitles?
   - Take highest quality automatically?
   - Prompt user to choose?

4. **Archive Retention**: Keep or delete downloaded ZIPs after extraction?
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
