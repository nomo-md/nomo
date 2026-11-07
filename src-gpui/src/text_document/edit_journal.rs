use super::{SegmentedEdit, TextDocumentError, TextDocumentResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BaselineIdentity {
    pub(super) path: String,
    pub(super) physical_len: u64,
    pub(super) modified_nanos: u64,
    // 长度与 mtime 可被外部工具原样保留；文件 ID 用来识别同路径上的原子替换。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) file_id: Option<String>,
    // Unix ctime 等不可由普通写入恢复的变更戳用于识别同 inode 的原地改写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) change_stamp: Option<String>,
}

impl BaselineIdentity {
    pub(super) fn read(path: &Path) -> TextDocumentResult<Self> {
        // 路径身份也通过句柄读取，Windows 才能获得稳定 file index 与 ChangeTime。
        let file = File::open(path)?;
        Self::read_file(path, &file)
    }

    /// 从已打开句柄读取身份，使 probe 字节与 baseline 绑定到同一个文件对象。
    pub(super) fn read_file(path: &Path, file: &File) -> TextDocumentResult<Self> {
        let metadata = file.metadata()?;
        let modified = metadata.modified().map_err(|error| {
            TextDocumentError::new(
                "file-identity-failed",
                format!("读取文件修改时间失败：{error}"),
            )
        })?;
        let modified_nanos = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| {
                TextDocumentError::new(
                    "file-identity-failed",
                    format!("文件修改时间早于 Unix epoch：{error}"),
                )
            })?
            .as_nanos() as u64;
        Ok(Self {
            path: path.to_string_lossy().into_owned(),
            physical_len: metadata.len(),
            modified_nanos,
            file_id: platform_file_id(file, &metadata)?,
            change_stamp: platform_change_stamp(file, &metadata)?,
        })
    }
}

#[cfg(unix)]
fn platform_file_id(
    _file: &File,
    metadata: &std::fs::Metadata,
) -> TextDocumentResult<Option<String>> {
    use std::os::unix::fs::MetadataExt;
    Ok(Some(format!("unix:{}:{}", metadata.dev(), metadata.ino())))
}

#[cfg(unix)]
fn platform_change_stamp(
    _file: &File,
    metadata: &std::fs::Metadata,
) -> TextDocumentResult<Option<String>> {
    use std::os::unix::fs::MetadataExt;
    Ok(Some(format!(
        "unix:{}:{}",
        metadata.ctime(),
        metadata.ctime_nsec()
    )))
}

#[cfg(windows)]
fn platform_file_id(
    file: &File,
    _metadata: &std::fs::Metadata,
) -> TextDocumentResult<Option<String>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: raw handle 在 file 借用期间有效，输出缓冲区类型与 API 要求一致。
    let result = unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) };
    if result == 0 {
        return Err(TextDocumentError::new(
            "file-identity-failed",
            format!(
                "读取 Windows 文件 ID 失败：{}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    let file_index = ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64;
    Ok(Some(format!(
        "windows:{}:{file_index}",
        info.dwVolumeSerialNumber
    )))
}

#[cfg(windows)]
fn platform_change_stamp(
    file: &File,
    _metadata: &std::fs::Metadata,
) -> TextDocumentResult<Option<String>> {
    use std::{mem::size_of, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
    };

    let mut info = FILE_BASIC_INFO::default();
    // SAFETY: raw handle 在 file 借用期间有效，输出缓冲区尺寸与 FILE_BASIC_INFO 完全一致。
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle().cast(),
            FileBasicInfo,
            (&mut info as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(TextDocumentError::new(
            "file-identity-failed",
            format!(
                "读取 Windows 文件 ChangeTime 失败：{}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    Ok(Some(format!("windows-change:{}", info.ChangeTime)))
}

#[cfg(not(any(unix, windows)))]
fn platform_file_id(
    _file: &File,
    _metadata: &std::fs::Metadata,
) -> TextDocumentResult<Option<String>> {
    Ok(None)
}

#[cfg(not(any(unix, windows)))]
fn platform_change_stamp(
    _file: &File,
    _metadata: &std::fs::Metadata,
) -> TextDocumentResult<Option<String>> {
    Ok(None)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JournalRecord {
    pub(super) base_revision: u64,
    pub(super) revision: u64,
    pub(super) edits: Vec<SegmentedEdit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) replacement_file: Option<JournalReplacementFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JournalReplacementFile {
    pub(super) path: String,
    pub(super) line_breaks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecoverySnapshot {
    pub(super) path: String,
    pub(super) revision: u64,
    pub(super) bom_len: u64,
}

#[derive(Debug)]
pub(super) struct RecoveryConflict {
    pub(super) journal_path: PathBuf,
    pub(super) snapshot: Option<RecoverySnapshot>,
    pub(super) records: Vec<JournalRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum JournalPayload {
    Header {
        version: u32,
        baseline: BaselineIdentity,
        #[serde(default)]
        persisted_revision: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recovery_snapshot: Option<RecoverySnapshot>,
    },
    Edit {
        record: JournalRecord,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalEnvelope {
    checksum: String,
    payload: JournalPayload,
}

#[derive(Debug)]
pub(super) struct EditJournal {
    path: PathBuf,
    baseline: BaselineIdentity,
    persisted_revision: u64,
    records: Vec<JournalRecord>,
    flushed_records: usize,
    flushed_file_len: u64,
    recovery_snapshot: Option<RecoverySnapshot>,
    header_dirty: bool,
}

impl EditJournal {
    pub(super) fn recovery_conflict_for_missing(
        path: &Path,
    ) -> TextDocumentResult<Option<RecoveryConflict>> {
        if !path.is_file() {
            return Ok(None);
        }
        let (_, _, records, _, snapshot) = read_journal(path)?;
        Ok(Some(RecoveryConflict {
            journal_path: path.to_path_buf(),
            snapshot,
            records,
        }))
    }

    pub(super) fn empty(path: PathBuf, baseline: BaselineIdentity) -> Self {
        Self {
            path,
            baseline,
            persisted_revision: 0,
            records: Vec::new(),
            flushed_records: 0,
            flushed_file_len: 0,
            recovery_snapshot: None,
            header_dirty: false,
        }
    }

    pub(super) fn open(
        path: PathBuf,
        baseline: BaselineIdentity,
    ) -> TextDocumentResult<(Self, u64, Vec<JournalRecord>, Option<RecoveryConflict>)> {
        let mut recovered = Vec::new();
        let mut retained_records = Vec::new();
        let mut persisted_revision = 0_u64;
        let mut flushed_file_len = 0_u64;
        let mut recovery_snapshot = None;
        let mut recovery_conflict = None;
        if path.is_file() {
            match read_journal(&path) {
                Ok((stored_baseline, stored_revision, records, valid_len, stored_snapshot))
                    if stored_baseline == baseline =>
                {
                    persisted_revision = stored_revision;
                    recovered = records
                        .iter()
                        .filter(|record| record.revision > stored_revision)
                        .cloned()
                        .collect();
                    retained_records = records;
                    flushed_file_len = valid_len;
                    recovery_snapshot = stored_snapshot;
                }
                Ok((_, _, records, _, stored_snapshot)) => {
                    // 冲突数据先保持原位；只有用户可读恢复文档物化成功后才清理内部资产。
                    recovery_conflict = Some(RecoveryConflict {
                        journal_path: path.clone(),
                        snapshot: stored_snapshot,
                        records,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        let journal = Self {
            path,
            baseline,
            persisted_revision,
            records: retained_records.clone(),
            flushed_records: retained_records.len(),
            flushed_file_len,
            recovery_snapshot,
            header_dirty: false,
        };
        Ok((journal, persisted_revision, recovered, recovery_conflict))
    }

    pub(super) fn record(&mut self, record: JournalRecord) {
        self.records.push(record);
    }

    pub(super) fn records(&self) -> &[JournalRecord] {
        &self.records
    }

    pub(super) fn needs_recovery_snapshot(&self) -> bool {
        self.recovery_snapshot
            .as_ref()
            .is_none_or(|snapshot| !Path::new(&snapshot.path).is_file())
    }

    pub(super) fn recovery_snapshot_path(&self) -> PathBuf {
        self.path.with_extension("snapshot")
    }

    pub(super) fn install_recovery_snapshot(&mut self, snapshot: RecoverySnapshot) {
        self.recovery_snapshot = Some(snapshot);
        self.header_dirty = true;
    }

    pub(super) fn has_records(&self) -> bool {
        !self.records.is_empty()
    }

    pub(super) fn referenced_assets(&self) -> HashSet<PathBuf> {
        self.records
            .iter()
            .filter_map(|record| record.replacement_file.as_ref())
            .map(|replacement| PathBuf::from(&replacement.path))
            .collect()
    }

    pub(super) fn flush(&mut self) -> TextDocumentResult<()> {
        if self.records.len() == self.flushed_records && self.path.is_file() && !self.header_dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if self.header_dirty && self.flushed_file_len > 0 {
            let temp_path = self.path.with_extension("journal.tmp");
            let mut file = File::create(&temp_path)?;
            write_payload(
                &mut file,
                JournalPayload::Header {
                    version: JOURNAL_VERSION,
                    baseline: self.baseline.clone(),
                    persisted_revision: self.persisted_revision,
                    recovery_snapshot: self.recovery_snapshot.clone(),
                },
            )?;
            for record in &self.records {
                write_payload(
                    &mut file,
                    JournalPayload::Edit {
                        record: record.clone(),
                    },
                )?;
            }
            file.sync_all()?;
            let flushed_file_len = file.stream_position()?;
            drop(file);
            replace_journal_file(&temp_path, &self.path)?;
            sync_parent_directory(&self.path)?;
            self.flushed_file_len = flushed_file_len;
            self.flushed_records = self.records.len();
            self.header_dirty = false;
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&self.path)?;
        // 上次写失败可能留下半条 JSON；重试必须先回到最后一次 sync 的确定边界。
        file.set_len(self.flushed_file_len)?;
        file.seek(SeekFrom::End(0))?;
        if self.flushed_file_len == 0 {
            write_payload(
                &mut file,
                JournalPayload::Header {
                    version: JOURNAL_VERSION,
                    baseline: self.baseline.clone(),
                    persisted_revision: self.persisted_revision,
                    recovery_snapshot: self.recovery_snapshot.clone(),
                },
            )?;
        }
        for record in &self.records[self.flushed_records..] {
            write_payload(
                &mut file,
                JournalPayload::Edit {
                    record: record.clone(),
                },
            )?;
        }
        file.sync_all()?;
        let flushed_file_len = file.stream_position()?;
        sync_parent_directory(&self.path)?;
        self.flushed_file_len = flushed_file_len;
        self.flushed_records = self.records.len();
        self.header_dirty = false;
        Ok(())
    }

    pub(super) fn prune(
        &mut self,
        persisted_revision: u64,
        baseline: BaselineIdentity,
        next_path: PathBuf,
        protected_assets: &HashSet<PathBuf>,
        bom_len: u64,
    ) -> TextDocumentResult<()> {
        let previous_path = self.path.clone();
        let previous_snapshot = self.recovery_snapshot.clone();
        let has_unpersisted_records = self
            .records
            .iter()
            .any(|record| record.revision > persisted_revision);
        let retention_floor = previous_snapshot
            .as_ref()
            .map(|snapshot| snapshot.revision.min(persisted_revision))
            .unwrap_or(persisted_revision);
        let retained: Vec<_> = self
            .records
            .iter()
            .filter(|record| has_unpersisted_records && record.revision > retention_floor)
            .cloned()
            .collect();
        let retained_assets: HashSet<_> = retained
            .iter()
            .filter_map(|record| record.replacement_file.as_ref())
            .map(|replacement| PathBuf::from(&replacement.path))
            .collect();
        let obsolete_replacements: Vec<_> = self
            .records
            .iter()
            .filter(|record| !has_unpersisted_records || record.revision <= retention_floor)
            .filter_map(|record| record.replacement_file.as_ref())
            .map(|replacement| PathBuf::from(&replacement.path))
            .filter(|path| !protected_assets.contains(path) && !retained_assets.contains(path))
            .collect();
        if retained.is_empty() {
            remove_obsolete_asset(&previous_path);
            if previous_path != next_path {
                remove_obsolete_asset(&next_path);
            }
            if let Some(snapshot) = &previous_snapshot {
                remove_obsolete_asset(Path::new(&snapshot.path));
            }
            let next_snapshot_path = next_path.with_extension("snapshot");
            if previous_snapshot
                .as_ref()
                .is_none_or(|snapshot| Path::new(&snapshot.path) != next_snapshot_path)
            {
                remove_obsolete_asset(&next_snapshot_path);
            }
            for replacement in obsolete_replacements {
                remove_obsolete_asset(&replacement);
            }
            self.path = next_path;
            self.records.clear();
            self.baseline = baseline;
            self.persisted_revision = persisted_revision;
            self.flushed_records = 0;
            self.flushed_file_len = 0;
            self.recovery_snapshot = None;
            self.header_dirty = false;
            return Ok(());
        }
        if let Some(parent) = next_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let next_snapshot_path = next_path.with_extension("snapshot");
        let recovery_snapshot = match previous_snapshot.as_ref() {
            Some(snapshot) if Path::new(&snapshot.path).is_file() => {
                if Path::new(&snapshot.path) != next_snapshot_path {
                    copy_file_durable(Path::new(&snapshot.path), &next_snapshot_path)?;
                }
                RecoverySnapshot {
                    path: next_snapshot_path.to_string_lossy().into_owned(),
                    revision: snapshot.revision,
                    bom_len: snapshot.bom_len,
                }
            }
            _ => {
                // 保存期间出现更高 revision 且尚未来得及建快照时，以刚落盘的 revision 为恢复起点。
                copy_file_durable(Path::new(&baseline.path), &next_snapshot_path)?;
                RecoverySnapshot {
                    path: next_snapshot_path.to_string_lossy().into_owned(),
                    revision: persisted_revision,
                    bom_len,
                }
            }
        };
        let temp_path = next_path.with_extension("journal.tmp");
        let mut file = File::create(&temp_path)?;
        write_payload(
            &mut file,
            JournalPayload::Header {
                version: JOURNAL_VERSION,
                baseline: baseline.clone(),
                persisted_revision,
                recovery_snapshot: Some(recovery_snapshot.clone()),
            },
        )?;
        for record in &retained {
            write_payload(
                &mut file,
                JournalPayload::Edit {
                    record: record.clone(),
                },
            )?;
        }
        file.sync_all()?;
        let flushed_file_len = file.stream_position()?;
        drop(file);
        replace_journal_file(&temp_path, &next_path)?;
        sync_parent_directory(&next_path)?;
        if previous_path != next_path {
            remove_obsolete_asset(&previous_path);
        }
        if let Some(snapshot) = previous_snapshot {
            if Path::new(&snapshot.path) != next_snapshot_path {
                remove_obsolete_asset(Path::new(&snapshot.path));
            }
        }
        for replacement in obsolete_replacements {
            remove_obsolete_asset(&replacement);
        }
        self.path = next_path;
        self.records = retained;
        self.baseline = baseline;
        self.persisted_revision = persisted_revision;
        self.flushed_records = self.records.len();
        self.flushed_file_len = flushed_file_len;
        self.recovery_snapshot = Some(recovery_snapshot);
        self.header_dirty = false;
        Ok(())
    }

    pub(super) fn remove(&mut self) -> TextDocumentResult<()> {
        // 先删除日志，确保“放弃修改”绝不会因清理任务临时文件失败而再次恢复。
        remove_file_if_exists(&self.path)?;
        remove_obsolete_asset(&self.path.with_extension("journal.tmp"));
        if let Some(snapshot) = self.recovery_snapshot.take() {
            remove_obsolete_asset(Path::new(&snapshot.path));
        }
        for replacement in self
            .records
            .iter()
            .filter_map(|record| record.replacement_file.as_ref())
        {
            remove_obsolete_asset(Path::new(&replacement.path));
        }
        self.records.clear();
        self.flushed_records = 0;
        self.flushed_file_len = 0;
        self.header_dirty = false;
        Ok(())
    }
}

pub(super) fn cleanup_recovery_conflict(conflict: &RecoveryConflict) {
    for path in std::iter::once(conflict.journal_path.clone())
        .chain(
            conflict
                .snapshot
                .iter()
                .map(|snapshot| PathBuf::from(&snapshot.path)),
        )
        .chain(
            conflict
                .records
                .iter()
                .filter_map(|record| record.replacement_file.as_ref())
                .map(|replacement| PathBuf::from(&replacement.path)),
        )
    {
        if let Err(error) = remove_file_if_exists(&path) {
            crate::app_logger::warn(
                "SegmentedDocument",
                &format!("清理已物化的冲突恢复资产失败：{error}"),
            );
        }
    }
}

fn copy_file_durable(source: &Path, target: &Path) -> TextDocumentResult<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = target.with_extension("snapshot.tmp");
    remove_file_if_exists(&temp)?;
    let mut source_file = File::open(source)?;
    let mut target_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    std::io::copy(&mut source_file, &mut target_file)?;
    target_file.sync_all()?;
    drop(target_file);
    replace_journal_file(&temp, target)?;
    sync_parent_directory(target)?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_parent_directory(path: &Path) -> TextDocumentResult<()> {
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn sync_parent_directory(_path: &Path) -> TextDocumentResult<()> {
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_obsolete_asset(path: &Path) {
    if let Err(error) = remove_file_if_exists(path) {
        crate::app_logger::warn(
            "SegmentedDocument",
            &format!("清理过期恢复资产失败（{}）：{error}", path.display()),
        );
    }
}

#[cfg(target_os = "windows")]
fn replace_journal_file(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let mut from: Vec<u16> = temp_path.as_os_str().encode_wide().collect();
    from.push(0);
    let mut to: Vec<u16> = target_path.as_os_str().encode_wide().collect();
    to.push(0);
    // SAFETY: 两个 UTF-16 路径均显式 NUL 结尾，缓冲区在调用期间有效；flags 要求覆盖并落盘。
    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_journal_file(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    std::fs::rename(temp_path, target_path)
}

fn read_journal(
    path: &Path,
) -> TextDocumentResult<(
    BaselineIdentity,
    u64,
    Vec<JournalRecord>,
    u64,
    Option<RecoverySnapshot>,
)> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut payloads = Vec::new();
    let mut valid_len = 0_u64;
    loop {
        line.clear();
        let count = reader.read_line(&mut line)?;
        if count == 0 {
            break;
        }
        if !line.ends_with('\n') {
            // 崩溃可能只留下最后一条不完整记录；已校验的前缀仍然可恢复。
            break;
        }
        let envelope: JournalEnvelope = serde_json::from_str(line.trim_end()).map_err(|error| {
            TextDocumentError::new("journal-corrupt", format!("恢复日志无法解析：{error}"))
        })?;
        let bytes = serde_json::to_vec(&envelope.payload).map_err(|error| {
            TextDocumentError::new("journal-corrupt", format!("恢复日志无法校验：{error}"))
        })?;
        if checksum(&bytes) != envelope.checksum {
            return Err(TextDocumentError::new(
                "journal-checksum-failed",
                "恢复日志校验失败",
            ));
        }
        payloads.push(envelope.payload);
        valid_len += count as u64;
    }

    let Some(JournalPayload::Header {
        version,
        baseline,
        persisted_revision,
        recovery_snapshot,
    }) = payloads.first().cloned()
    else {
        return Err(TextDocumentError::new(
            "journal-corrupt",
            "恢复日志缺少头部",
        ));
    };
    if version != JOURNAL_VERSION {
        return Err(TextDocumentError::new(
            "journal-version-unsupported",
            format!("不支持的恢复日志版本：{version}"),
        ));
    }
    let records = payloads
        .into_iter()
        .skip(1)
        .filter_map(|payload| match payload {
            JournalPayload::Edit { record } => Some(record),
            JournalPayload::Header { .. } => None,
        })
        .collect();
    Ok((
        baseline,
        persisted_revision,
        records,
        valid_len,
        recovery_snapshot,
    ))
}

fn write_payload(file: &mut File, payload: JournalPayload) -> TextDocumentResult<()> {
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|error| TextDocumentError::new("journal-serialize-failed", error.to_string()))?;
    let envelope = JournalEnvelope {
        checksum: checksum(&payload_bytes),
        payload,
    };
    serde_json::to_writer(&mut *file, &envelope)
        .map_err(|error| TextDocumentError::new("journal-write-failed", error.to_string()))?;
    file.write_all(b"\n")?;
    Ok(())
}

fn checksum(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{replace_journal_file, BaselineIdentity, EditJournal, JournalRecord};
    use crate::text_document::SegmentedEdit;
    use std::{
        fs,
        io::Write,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn truncated_tail_recovers_verified_prefix() {
        let root = unique_test_dir("truncated");
        fs::create_dir_all(&root).expect("root");
        let document = root.join("sample.txt");
        fs::write(&document, "base").expect("document");
        let identity = BaselineIdentity::read(&document).expect("identity");
        let journal_path = root.join("sample.journal");
        let (mut journal, _, _, _) =
            EditJournal::open(journal_path.clone(), identity.clone()).expect("journal");
        journal.record(sample_record());
        journal.flush().expect("flush");
        fs::OpenOptions::new()
            .append(true)
            .open(&journal_path)
            .expect("open tail")
            .write_all(b"{\"partial\"")
            .expect("partial tail");

        let (_, _, recovered, _) =
            EditJournal::open(journal_path, identity).expect("recover prefix");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].revision, 1);
        cleanup(root);
    }

    #[test]
    fn checksum_failure_is_reported_instead_of_silently_recovered() {
        let root = unique_test_dir("checksum");
        fs::create_dir_all(&root).expect("root");
        let document = root.join("sample.txt");
        fs::write(&document, "base").expect("document");
        let identity = BaselineIdentity::read(&document).expect("identity");
        let journal_path = root.join("sample.journal");
        let (mut journal, _, _, _) =
            EditJournal::open(journal_path.clone(), identity.clone()).expect("journal");
        journal.record(sample_record());
        journal.flush().expect("flush");

        let content = fs::read_to_string(&journal_path).expect("content");
        let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
        let mut envelope: serde_json::Value = serde_json::from_str(&lines[1]).expect("envelope");
        envelope["checksum"] = serde_json::Value::String("00".into());
        lines[1] = serde_json::to_string(&envelope).expect("serialize");
        fs::write(&journal_path, format!("{}\n", lines.join("\n"))).expect("corrupt");

        let error = EditJournal::open(journal_path, identity).expect_err("checksum error");
        assert_eq!(error.code, "journal-checksum-failed");
        cleanup(root);
    }

    #[test]
    fn journal_replacement_overwrites_an_existing_target() {
        let root = unique_test_dir("replace-existing");
        fs::create_dir_all(&root).expect("root");
        let target = root.join("target.journal");
        let temp = root.join("target.journal.tmp");
        fs::write(&target, "old").expect("old target");
        fs::write(&temp, "new").expect("new temp");

        replace_journal_file(&temp, &target).expect("replace existing journal");

        assert_eq!(fs::read_to_string(&target).expect("target"), "new");
        assert!(!temp.exists());
        cleanup(root);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_change_time_detects_in_place_rewrite_with_restored_mtime() {
        let root = unique_test_dir("windows-change-time");
        fs::create_dir_all(&root).expect("root");
        let document = root.join("sample.txt");
        fs::write(&document, "AAAA").expect("document");
        let before = BaselineIdentity::read(&document).expect("before identity");
        let modified = fs::metadata(&document)
            .expect("before metadata")
            .modified()
            .expect("before mtime");

        // NTFS 的 ChangeTime 可能在同一系统时钟粒度内相同；先跨过该间隔再验证改写。
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut rewritten = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&document)
            .expect("rewrite handle");
        rewritten.write_all(b"BBBB").expect("rewrite content");
        rewritten.sync_all().expect("sync rewrite");
        drop(rewritten);
        fs::OpenOptions::new()
            .write(true)
            .open(&document)
            .expect("restore mtime handle")
            .set_modified(modified)
            .expect("restore mtime");

        let after = BaselineIdentity::read(&document).expect("after identity");
        assert_eq!(
            before.file_id, after.file_id,
            "in-place write keeps file id"
        );
        assert_eq!(before.physical_len, after.physical_len);
        assert_eq!(before.modified_nanos, after.modified_nanos);
        assert_ne!(before.change_stamp, after.change_stamp);
        assert_ne!(before, after);
        cleanup(root);
    }

    fn sample_record() -> JournalRecord {
        JournalRecord {
            base_revision: 0,
            revision: 1,
            edits: vec![SegmentedEdit {
                from_byte: 4,
                to_byte: 4,
                inserted_text: " edit".into(),
            }],
            replacement_file: None,
        }
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("nomo-journal-{name}-{nonce}"))
    }

    fn cleanup(path: PathBuf) {
        let _ = fs::remove_dir_all(path);
    }
}
