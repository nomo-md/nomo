# Android PR #44: storage, search and acceptance

This is a preview feature, not a production Android release. Keep PR #44 in Draft until automated checks and phone acceptance pass. Final merge requires the maintainer's explicit confirmation of the tested commit.

## Behavior and data ownership

- Text and HTTPS links shared to Nomo are imported as editable text. Nomo does not fetch a shared URL or execute its protocol. The cold-start and running-activity entry points use the same text normalization. A delivery gets a stable document name; replay does not overwrite edits.
- `content://` and `file://` inputs are copied to the app's internal data directory (`incoming/`). Saving edits changes that copy, not the provider's original file. Imports are staged, synced and atomically published without replacing another document of the same name.
- Imported documents have no age-based expiration. Clearing Android's cache does not remove durable imports. Uninstalling Nomo or clearing **app data/storage** still removes app-owned files; export anything important first. See [Android app-specific storage](https://developer.android.com/training/data-storage/app-specific).
- On startup, legacy `cache/incoming` files are copied before recent entries, workspace paths, pending-open records and snapshot indexes are updated. Existing migrated copies are reused, not overwritten. Failed copies retain the original file and reference; failed configuration persistence retains the original configuration. Legacy originals are not deleted by migration. If a migration warning appears, do **not** clear cache until migration succeeds.
- Android logs redact URL/path suffixes and free-form string metadata before buffering, DevTools and native IPC. Native log output has the same path/URL boundary. Event labels, numeric metrics and boolean state remain available; desktop logging is unchanged.
- Replay recognition uses a fingerprint from saved Activity state, not an external Intent's claimed processing flag. Android path comparison preserves case, so differently-cased import names cannot silently select the same tab.
- After syncing master, mobile file opening stays in the current activity and does not invoke the desktop-only open-target/window registry. The desktop routing remains unchanged.

## Search contract

- Search covers open tabs and recent Android documents, never a recursive disk walk. Filename filtering is immediate; content scanning starts after 300ms of no input.
- A single Rust worker scans closed documents read-only, in 64KiB chunks, without opening editing sessions. Encoding detection preserves UTF-8/BOM, UTF-16 BOM and GBK behavior. An initial streamed UTF-8 validation pass may be required to select the encoding.
- Open TXT/JSON uses the current revision's segmented snapshot, including unsaved edits. Open Markdown sends at most 32,768 UTF-16 code units per message to a Web Worker, with one chunk in flight; it does not copy the entire tab into the worker.
- Matching remains case-insensitive with Unicode NFKD normalization and supports read-boundary matches. Each document returns only the first excerpt, at most 160 Unicode characters, with original-text UTF-16 highlight offsets. Only excerpts/errors are retained in the drawer.
- Changing the query, closing the drawer or destroying the component cancels prior work. Task IDs and document versions reject stale results; cancellation is checked before the next document/read. File changes and read errors are distinct from no match.
- New IPC: `start_mobile_document_search`, `cancel_mobile_document_search`, and the `nomo://mobile-search` progress/result event. Existing document-open events and normal read/write commands remain intact.

## Automated checks

The `PR validation` workflow runs on `pull_request`, checks out the exact head SHA, has `contents: read`, does not retain checkout credentials, and never receives release/signing secrets. Independent jobs cover:

1. Frontend build, targeted Vitest regressions, and `pnpm check` (no ignored failures).
2. Windows and macOS compilation, targeted Rust import/search regressions, and the existing segmented save regression.
3. Android ARM64 debug APK build, shared-text JVM tests, package/label/ABI verification, and SHA-256 output.

Rust tests cover atomic publication, name collisions, unchanged provider originals, idempotent old-cache migration, recent/workspace/pending/snapshot references, copy/config failures, reopening after old-cache deletion, 200 documents, a streamed 100MiB tail match, Unicode/cross-chunk/encoding cases, cancellation and dirty TXT/JSON snapshots. Import/search tests reserve distinct temporary directories atomically, with a parallel isolation regression; timestamps alone are not unique on Windows. Vitest covers debounce, cancellation, stale task/version filtering and unsaved Markdown routing. These tests do not replace phone acceptance.

The original PR baseline had two type-check errors in `pendingInlineMark.test.ts` (MouseEvent versus PointerEvent). Its click fixture now supplies the complete pointer-event contract on a dispatchable jsdom mouse event; editor production code and existing assertions are unchanged. This suite is included in the targeted CI regressions. Type-check failures are never hidden or waived, and an absent/skipped/approval-pending job is not a passing check.

## Preview APK

CI artifacts are named `nomo-pr44-arm64-<head SHA>`. Each archive contains:

- an offline-capable ARM64 debug APK with app ID `com.nomo.desktop.pr44` and label **Nomo PR44**, isolated from the normal app;
- `BUILD.txt` with the exact source commit;
- `SHA256SUMS.txt` and this checklist.
- a Chinese phone checklist and a compressed fixture with 199 small documents and one exact 100MiB TXT (cross-boundary and tail markers).

It uses an ephemeral CI debug key, not a production key. Separate CI runs may use different debug keys: Android can require uninstalling the previous **preview** before installation. Export preview documents first; uninstall deletes that preview's data. Do not uninstall the normal app.

## Phone acceptance (user completes)

Record APK commit, SHA-256, Android version, device model and result for every row. Use disposable test documents, not the only copy of important data.

| Check | Expected result | Result |
| --- | --- | --- |
| Install preview beside normal Nomo | Both apps and their data coexist | Pending |
| Share ordinary text with app stopped, then with app running | One editable document per share, no duplicate tabs | Pending |
| Repeat with Chinese and multiline text | Text and line breaks preserved | Pending |
| Repeat with HTTPS URL containing query and fragment | Exact text, no network fetch/browser launch | Pending |
| Empty text, unsupported file and denied provider access | Visible error; no partially imported document | Pending |
| Import from WeChat/file provider; import same filename twice | Independent copies, original provider file untouched | Pending |
| Edit/save import, stop/restart preview | Saved content retained | Pending |
| Clear **cache only**, reopen recent imported document | Document and saved content retained | Pending |
| Legacy-cache migration fixture, restart twice | Same durable copy; recent/workspace/snapshots work; no overwritten edits | Pending |
| Search 200 recent documents including a 100MiB file | Filename filter immediate; content progress visible; drawer remains usable | Pending |
| Match near end and across a 64KiB boundary | First excerpt/highlight correct | Pending |
| Quickly replace query, then close/reopen drawer | Old results never overwrite the latest query | Pending |
| Search unsaved Markdown, TXT and JSON edits | Current unsaved content can be found | Pending |
| Missing/unreadable recent file versus no match | Read error distinguishable from zero matches | Pending |

Desktop validation in this PR is compilation and related automated regressions only, not Windows/macOS device acceptance. If phone validation fails, return to fixes and regenerate/retest the APK. Do not mark Ready or merge using an older APK's results.
