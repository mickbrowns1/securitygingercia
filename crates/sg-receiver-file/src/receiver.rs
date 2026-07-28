use crate::config::{FileLogConfig, StartAt};
use crate::discovery::{build_globset, discover_files, split_pattern, DiscoveryPattern};
use async_trait::async_trait;
use globset::GlobSet;
use sg_checkpoint::{identity_of, CheckpointStore, FileIdentity};
use sg_core::{Event, Receiver, SgError};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct TailState {
    path: PathBuf,
    /// Total bytes already read from the file (complete lines + trailing
    /// partial fragment) -- this is what gets persisted as the checkpoint
    /// offset and what `SeekFrom::Start` resumes from.
    offset: u64,
    /// Bytes read after `offset` on a previous tick that didn't yet form
    /// a complete line; prepended to the next read.
    partial: Vec<u8>,
}

pub struct FileLogReceiver {
    name: String,
    config: FileLogConfig,
}

impl FileLogReceiver {
    pub fn new(name: impl Into<String>, config: FileLogConfig) -> Self {
        Self {
            name: name.into(),
            config,
        }
    }
}

#[async_trait]
impl Receiver for FileLogReceiver {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(
        self: Box<Self>,
        tx: mpsc::Sender<Event>,
        shutdown: CancellationToken,
    ) -> Result<(), SgError> {
        let mut checkpoint = CheckpointStore::load(&self.config.checkpoint_file)?;
        let includes: Vec<DiscoveryPattern> =
            self.config.include.iter().map(|p| split_pattern(p)).collect();
        let excludes: GlobSet = build_globset(&self.config.exclude)
            .map_err(|e| SgError::Config(format!("{}: {e}", self.name)))?;

        let mut states: HashMap<FileIdentity, TailState> = HashMap::new();
        let mut ticker = tokio::time::interval(self.config.poll_interval);

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    if !self.tick(&includes, &excludes, &mut states, &mut checkpoint, &tx).await {
                        // Channel closed -- downstream pipeline is gone.
                        return Ok(());
                    }
                }
            }
        }

        let _ = checkpoint.flush();
        Ok(())
    }
}

impl FileLogReceiver {
    /// Returns `false` if the send channel closed and the receiver should
    /// stop.
    async fn tick(
        &self,
        includes: &[DiscoveryPattern],
        excludes: &GlobSet,
        states: &mut HashMap<FileIdentity, TailState>,
        checkpoint: &mut CheckpointStore,
        tx: &mpsc::Sender<Event>,
    ) -> bool {
        let files = discover_files(includes, excludes);
        let mut seen = std::collections::HashSet::new();

        for path in &files {
            let Ok(meta) = std::fs::metadata(path) else {
                continue;
            };
            let identity = identity_of(&meta);
            seen.insert(identity);

            let is_new = !states.contains_key(&identity);
            if is_new {
                let start_offset = checkpoint.get(identity).unwrap_or(match self.config.start_at {
                    StartAt::Beginning => 0,
                    StartAt::End => meta.len(),
                });
                states.insert(
                    identity,
                    TailState {
                        path: path.clone(),
                        offset: start_offset,
                        partial: Vec::new(),
                    },
                );
                tracing::debug!(receiver = %self.name, path = %path.display(), start_offset, "tailing new file");
            }

            let state = states.get_mut(&identity).unwrap();
            state.path = path.clone();

            // In-place truncation (same inode, shrunk size): resume from 0.
            if meta.len() < state.offset {
                tracing::debug!(receiver = %self.name, path = %path.display(), "file truncated, resetting offset");
                state.offset = 0;
                state.partial.clear();
            }

            let Ok(mut file) = std::fs::File::open(path) else {
                continue;
            };
            if file.seek(SeekFrom::Start(state.offset)).is_err() {
                continue;
            }
            let mut new_bytes = Vec::new();
            if file.read_to_end(&mut new_bytes).is_err() || new_bytes.is_empty() {
                continue;
            }

            state.partial.extend_from_slice(&new_bytes);
            state.offset += new_bytes.len() as u64;

            let mut consumed = 0usize;
            while let Some(rel_pos) = state.partial[consumed..].iter().position(|&b| b == b'\n') {
                let line_end = consumed + rel_pos;
                let mut line = state.partial[consumed..line_end].to_vec();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                consumed = line_end + 1;

                let mut event = Event::new(bytes::Bytes::from(line));
                event
                    .resource
                    .insert("file.path".to_string(), serde_json::json!(path.display().to_string()));
                event
                    .resource
                    .insert("receiver".to_string(), serde_json::json!(self.name.clone()));

                if tx.send(event).await.is_err() {
                    return false;
                }
            }
            state.partial.drain(0..consumed);

            checkpoint.set(identity, &state.path, state.offset);
        }

        // Evict identities that are gone for good: either the tracked path
        // no longer exists, or it does exist but now belongs to a
        // different file (e.g. logrotate recreated "app.log" under the
        // same name with a fresh inode -- the old inode's progress is
        // done and safe to drop).
        let stale: Vec<FileIdentity> = states
            .iter()
            .filter(|(id, _)| !seen.contains(*id))
            .filter(|(id, st)| match std::fs::metadata(&st.path) {
                Err(_) => true,
                Ok(meta) => identity_of(&meta) != **id,
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            states.remove(&id);
        }
        checkpoint.retain(|id| states.contains_key(&id));

        if let Err(e) = checkpoint.flush() {
            tracing::warn!(receiver = %self.name, error = %e, "failed to flush checkpoint");
        }

        true
    }
}
