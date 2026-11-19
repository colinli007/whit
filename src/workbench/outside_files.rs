//! Files open from outside the repository, such as `settings.json`, follow
//! the disk as the repository's do (`file-editor`「多标签」, design 7): the
//! folders they are in are watched, not recursively, while such a tab is
//! open; a change reaches the file's buffer, which reloads, keeps unsaved
//! edits for a conflict on save, or marks the file deleted.
use std::{collections::BTreeSet, path::PathBuf};

use gpui_kit::*;

use super::{Workbench, editor_group::TabKey};
use crate::watcher::{DirWatcher, FileBatch};

/// The watcher of the folders of files open from outside the repository.
pub(super) struct OutsideWatcher {
    watcher: DirWatcher,
    folders: BTreeSet<PathBuf>,
    _batches: Task<()>,
}

impl OutsideWatcher {
    #[cfg(test)]
    pub(super) fn folders(&self) -> &BTreeSet<PathBuf> {
        &self.folders
    }
}

impl Workbench {
    /// The folders of the open files outside the repository.
    fn outside_folders(&self) -> BTreeSet<PathBuf> {
        let root = self.repository.as_ref().map(|repository| repository.root.as_path());
        self.editors
            .group
            .tabs()
            .iter()
            .filter_map(|key| match key {
                TabKey::File(path) => Some(path),
                TabKey::Diff { .. } | TabKey::Settings => None,
            })
            .filter(|path| root.is_none_or(|root| !path.starts_with(root)))
            .filter_map(|path| path.parent().map(ToOwned::to_owned))
            .collect()
    }

    /// Watches exactly the folders of the open files outside the
    /// repository, after tabs opened or closed or the repository changed.
    pub(super) fn follow_outside_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.outside_folders();
        let watching = match &mut self.outside_watcher {
            Some(watching) => watching,
            None if wanted.is_empty() => return,
            None => match Self::start_outside_watcher(window, cx) {
                Some(watching) => self.outside_watcher.insert(watching),
                None => return,
            },
        };
        for folder in wanted.difference(&watching.folders) {
            if let Err(error) = watching.watcher.watch(folder) {
                log::warn!("cannot watch {}: {error}", folder.display());
            }
        }
        for folder in watching.folders.difference(&wanted) {
            if let Err(error) = watching.watcher.unwatch(folder) {
                log::warn!("cannot stop watching {}: {error}", folder.display());
            }
        }
        watching.folders = wanted;
    }

    fn start_outside_watcher(window: &mut Window, cx: &mut Context<Self>) -> Option<OutsideWatcher> {
        let (watcher, batches) = match DirWatcher::new() {
            Ok(watching) => watching,
            Err(error) => {
                log::error!("cannot watch files outside the repository: {error}");
                return None;
            }
        };
        Some(OutsideWatcher {
            watcher,
            folders: BTreeSet::new(),
            _batches: Self::take_outside_batches(batches, window, cx),
        })
    }

    /// Not in unit tests: their scheduler refuses wake-ups from the
    /// watcher's thread, so the batches are dropped (which ends the thread)
    /// and tests hand their own to `outside_files_changed`.
    #[cfg(test)]
    fn take_outside_batches(
        _: async_channel::Receiver<FileBatch>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Task<()> {
        Task::ready(())
    }

    #[cfg(not(test))]
    fn take_outside_batches(
        batches: async_channel::Receiver<FileBatch>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(batch) = batches.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.outside_files_changed(&batch, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    /// Checks the open files outside the repository that the batch names.
    pub(super) fn outside_files_changed(
        &mut self,
        batch: &FileBatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.repository.as_ref().map(|repository| repository.root.clone());
        let changed: Vec<_> = self
            .editors
            .buffers
            .iter()
            .filter(|(path, _)| root.as_ref().is_none_or(|root| !path.starts_with(root)))
            .filter(|(path, _)| batch.rescan || batch.paths.contains(path.as_ref()))
            .map(|(_, open)| open.buffer.clone())
            .collect();
        for buffer in changed {
            buffer.update(cx, |buffer, cx| buffer.disk_changed(window, cx));
        }
    }
}
