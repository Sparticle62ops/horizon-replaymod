//! Zip container change management: 1:1 Rust port of ReplayStudio's
//! ZipReplayFile write/remove/save/saveTo merge logic, recording CRC hash
//! invalidation + rewrite, and removed-entry tombstones (LGPL-3.0-or-later).
//! This file: Copyright (c) 2026 sparticle, LGPL-3.0-or-later.
//!
//! The zip wire format itself (ZipFile/ZipOutputStream, and the on-disk
//! cache directory with its GZIPped copies) is a Horizon-integration seam
//! behind McprArchive; this layer owns the semantics above it. The cache
//! invalidation on recording rewrite is modeled on the in-memory base map.
use crate::mcpr::*;
use crate::replayfile::crc32;
use std::collections::{BTreeMap, BTreeSet};

/// ZipReplayFile over an opened archive: base = the zip's entries at open.
pub struct ZipReplayFile<A: McprArchive> {
    pub archive: A,
    base: BTreeMap<String, Vec<u8>>,
    /// Upstream changedEntries: entry -> bytes (upstream uses temp files;
    /// buffered in memory here, semantically identical).
    changed_entries: BTreeMap<String, Vec<u8>>,
    /// Upstream removedEntries tombstones.
    removed_entries: BTreeSet<String>,

}

impl<A: McprArchive> ZipReplayFile<A> {
    /// Upstream open(): snapshot the zip's entries.
    pub fn open(archive: A) -> Self {
        let base: BTreeMap<String, Vec<u8>> = archive.list_entries()
            .into_iter()
            .filter_map(|name| archive.read_entry(&name).map(|b| (name, b)))
            .collect();
        Self { archive, base, changed_entries: BTreeMap::new(), removed_entries: BTreeSet::new() }
    }

    /// Upstream get(): changed copy first, else the base entry.
    pub fn get(&self, entry: &str) -> Option<Vec<u8>> {
        self.changed_entries.get(entry).cloned().or_else(|| self.base.get(entry).cloned())
    }

    /// Upstream write(): the change lands in changedEntries; a tombstone is
    /// lifted. For ENTRY_RECORDING the old hash is invalidated immediately,
    /// a CRC runs over everything written, and closing stores the new hash.
    pub fn write(&mut self, entry: &str) -> ZipWriter {
        self.removed_entries.remove(entry);
        if entry == ENTRY_RECORDING {
            // Upstream: immediately invalidate the old hash in case we
            // crash during writing.
            self.changed_entries.insert(ENTRY_RECORDING_HASH.to_string(), b"invalid".to_vec());
        }
        ZipWriter { file_entry: entry.to_string(), buf: Vec::new(), len_written: 0 }
    }

    /// Close half of upstream's anonymous OutputStream: folds the bytes into
    /// changedEntries and, for the recording, rewrites the CRC hash entry.
    pub fn finish_write(&mut self, w: ZipWriter) {
        if w.file_entry == ENTRY_RECORDING {
            let crc = crc32(&w.buf);
            self.changed_entries.insert(w.file_entry.clone(), w.buf);
            self.changed_entries.insert(ENTRY_RECORDING_HASH.to_string(), crc.to_string().into_bytes());
            // Upstream invalidates the on-disk cache here; ours is the base
            // snapshot, refreshed at save().
        } else {
            self.changed_entries.insert(w.file_entry, w.buf);
        }
    }

    /// Upstream remove(): drop the changed copy, tombstone the entry.
    pub fn remove(&mut self, entry: &str) {
        self.changed_entries.remove(entry);
        self.removed_entries.insert(entry.to_string());
    }

    /// Upstream save(): no-op with no changes; otherwise the merged entry
    /// set is written back through the archive (upstream: temp file + move).
    pub fn save(&mut self) {
        if self.changed_entries.is_empty() && self.removed_entries.is_empty() {
            return; // Upstream: no changes, no need to save
        }
        self.save_to_merged();
    }

    /// Upstream saveTo(target): original entries in order, skipping changed
    /// and removed, then every changed entry appended.
    fn save_to_merged(&mut self) {
        let merged: Vec<(String, Vec<u8>)> = self.base.iter()
            .filter(|(name, _)| !self.changed_entries.contains_key(*name) && !self.removed_entries.contains(*name))
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .chain(self.changed_entries.iter().map(|(n, b)| (n.clone(), b.clone())))
            .collect();
        self.base = merged.into_iter().collect();
        // Persist through the archive seam (upstream writes a fresh zip and
        // moves it over the output file): remove everything, write the merge.
        let mut names = self.archive.list_entries();
        names.sort();
        for name in names { self.archive.remove_entry(&name); }
        for (name, bytes) in self.base.clone() { self.archive.write_entry(&name, &bytes); }
        self.changed_entries.clear();
        self.removed_entries.clear();
    }

    /// Read-back convenience mirroring get() after save().
    pub fn entry_names(&self) -> Vec<String> { self.base.keys().cloned().collect() }

    /// Seed the base snapshot directly (test-only in upstream terms: upstream
    /// constructs with an existing ZipFile).
    pub fn seed_base(&mut self, entry: &str, bytes: &[u8]) {
        self.base.insert(entry.to_string(), bytes.to_vec());
    }
}

/// Buffered writer handed out by write(); fold with finish_write.
pub struct ZipWriter { file_entry: String, buf: Vec<u8>, len_written: usize }
impl ZipWriter {
    pub fn write_bytes(&mut self, b: &[u8]) { self.buf.extend_from_slice(b); self.len_written += b.len(); }
    pub fn len(&self) -> usize { self.len_written }
}
