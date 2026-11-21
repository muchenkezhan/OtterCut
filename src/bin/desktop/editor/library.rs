use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::SystemTime;

use eframe::egui;
use takeseam::PRODUCT_NAME;

use super::media::{MediaInfo, MediaTools};

pub(super) const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aiff", "aif",
];

pub(super) fn has_audio_extension(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        AUDIO_EXTENSIONS
            .iter()
            .any(|audio| extension.eq_ignore_ascii_case(audio))
    })
}

pub(super) struct Library {
    directory: PathBuf,
    pub paths: Vec<PathBuf>,
    pub selected: Option<usize>,
    dismissed: HashSet<PathBuf>,
    info: HashMap<PathBuf, Result<MediaInfo, String>>,
    pending: HashSet<PathBuf>,
    probe: Option<Probe>,
}

struct Probe {
    requests: mpsc::Sender<PathBuf>,
    results: mpsc::Receiver<(PathBuf, Result<MediaInfo, String>)>,
}

impl Library {
    pub(super) fn new(directory: PathBuf, tools: Option<&MediaTools>, ctx: &egui::Context) -> Self {
        let probe = tools.and_then(|tools| {
            let tools = tools.clone();
            let ctx = ctx.clone();
            let (requests, incoming) = mpsc::channel::<PathBuf>();
            let (results, outgoing) = mpsc::channel();
            thread::Builder::new()
                .name(format!("{PRODUCT_NAME} media probe"))
                .spawn(move || {
                    while let Ok(path) = incoming.recv() {
                        let info = tools.probe(&path).map_err(|error| format!("{error:#}"));
                        if results.send((path, info)).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                })
                .ok()?;
            Some(Probe {
                requests,
                results: outgoing,
            })
        });
        let mut library = Self {
            directory,
            paths: Vec::new(),
            selected: None,
            dismissed: HashSet::new(),
            info: HashMap::new(),
            pending: HashSet::new(),
            probe,
        };
        library.refresh_recordings();
        library
    }

    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(super) fn selected_path(&self) -> Option<&Path> {
        self.selected
            .and_then(|index| self.paths.get(index))
            .map(PathBuf::as_path)
    }

    pub(super) fn info(&self, path: &Path) -> Option<MediaInfo> {
        self.info
            .get(path)
            .and_then(|item| item.as_ref().ok())
            .copied()
    }

    pub(super) fn is_audio(&self, path: &Path) -> bool {
        self.info(path).map_or_else(
            || has_audio_extension(path),
            |info| info.has_audio && !info.has_video,
        )
    }

    pub(super) fn insert(&mut self, path: PathBuf, select: bool) {
        self.dismissed.remove(&path);
        let index = match self.paths.iter().position(|existing| *existing == path) {
            Some(index) => index,
            None => {
                let index = self.paths.len();
                self.paths.push(path);
                index
            }
        };
        if select || self.selected.is_none() {
            self.selected = Some(index);
        }
    }

    pub(super) fn remove(&mut self, path: &Path) -> bool {
        let Some(index) = self.paths.iter().position(|existing| existing == path) else {
            return false;
        };
        self.dismissed.insert(self.paths.remove(index));
        self.selected = if self.paths.is_empty() {
            None
        } else {
            self.selected.map(|selected| {
                let shifted = selected - usize::from(selected > index);
                shifted.min(self.paths.len() - 1)
            })
        };
        true
    }

    pub(super) fn refresh_recordings(&mut self) {
        let Ok(files) = fs::read_dir(&self.directory) else {
            return;
        };
        let recording_prefix = format!("{PRODUCT_NAME}-");
        let mut recordings: Vec<_> = files
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.ends_with(".mp4")
                    || !(name.starts_with(recording_prefix.as_str())
                        || name.starts_with("TakeSeam-"))
                {
                    return None;
                }
                let metadata = entry.metadata().ok()?;
                metadata.is_file().then_some((
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    entry.path(),
                ))
            })
            .collect();
        recordings.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.0));
        for (_, path) in recordings {
            if !self.dismissed.contains(&path) {
                self.insert(path, false);
            }
        }
    }

    pub(super) fn poll(&mut self) {
        if let Some(probe) = &self.probe {
            for (path, info) in probe.results.try_iter() {
                self.pending.remove(&path);
                self.info.insert(path, info);
            }
        }
    }

    pub(super) fn request(&mut self, path: &Path) -> Option<Result<MediaInfo, &str>> {
        if self.info.contains_key(path) {
            return self
                .info
                .get(path)
                .map(|entry| entry.as_ref().copied().map_err(String::as_str));
        }
        let Some(probe) = &self.probe else {
            return Some(Err("FFmpeg 不可用"));
        };
        if !self.pending.contains(path) {
            if probe.requests.send(path.to_owned()).is_err() {
                return Some(Err("素材读取任务已停止"));
            }
            self.pending.insert(path.to_owned());
        }
        None
    }

    pub(super) fn retry(&mut self, path: &Path) {
        if self.info.get(path).is_some_and(Result::is_err) {
            self.info.remove(path);
        }
    }

    pub(super) fn insert_with_info(&mut self, path: PathBuf, info: MediaInfo, select: bool) {
        self.insert(path.clone(), select);
        self.info.insert(path, Ok(info));
    }

    #[cfg(test)]
    pub(super) fn add_fixture(&mut self, path: PathBuf, info: MediaInfo) {
        self.insert_with_info(path, info, false);
    }
}

pub(super) fn filename(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismissed_recordings_stay_hidden_until_reimported() {
        let directory = tempfile::tempdir().unwrap();
        let recorded = directory.path().join(format!("{PRODUCT_NAME}-001.mp4"));
        fs::write(&recorded, b"recording").unwrap();
        let ctx = egui::Context::default();
        let mut library = Library::new(directory.path().to_owned(), None, &ctx);

        assert_eq!(library.selected_path(), Some(recorded.as_path()));
        assert!(library.remove(&recorded));
        assert!(library.paths.is_empty());
        assert_eq!(library.selected_path(), None);
        assert_eq!(fs::read(&recorded).unwrap().as_slice(), b"recording");

        library.refresh_recordings();
        assert!(library.paths.is_empty());

        library.insert(recorded.clone(), true);
        library.refresh_recordings();
        assert_eq!(library.paths.as_slice(), std::slice::from_ref(&recorded));
        assert_eq!(library.selected_path(), Some(recorded.as_path()));
    }

    #[test]
    fn removing_material_keeps_metadata_and_maintains_selection() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.mp4");
        let middle = directory.path().join("middle.mp4");
        let last = directory.path().join("last.mp4");
        let ctx = egui::Context::default();
        let mut library = Library::new(directory.path().to_owned(), None, &ctx);
        library.insert(first.clone(), false);
        library.insert_with_info(
            middle.clone(),
            MediaInfo {
                duration_us: 2_000_000,
                width: 640,
                height: 360,
                has_audio: true,
                has_video: true,
            },
            true,
        );
        library.insert(last.clone(), false);
        library.pending.insert(last.clone());

        assert!(library.remove(&first));
        assert_eq!(library.selected_path(), Some(middle.as_path()));
        assert!(library.remove(&middle));
        assert_eq!(library.selected_path(), Some(last.as_path()));
        assert_eq!(
            library.request(&middle).unwrap().unwrap().duration_us,
            2_000_000
        );
        assert!(library.remove(&last));
        assert!(library.pending.contains(&last));
        assert_eq!(library.selected_path(), None);
        assert!(!library.remove(&last));
    }
}
