//! Autosave: every few minutes a snapshot of a modified model is written, on a background
//! thread, to its own file in the config folder (never over the working file). Files left
//! there by a session that ended badly are offered for recovery at the next start.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant, SystemTime};

use tri_doc::Document;

/// Folder of the autosave files: `<config>/triceratops/autosave`.
pub fn dir() -> Option<PathBuf> {
    crate::app::options_path().and_then(|p| p.parent().map(|d| d.join("autosave")))
}

/// Autosave file of a model in `dir`: `<stem>.autosave.3dm`, `Untitled` for a new model.
pub fn path_in(dir: &Path, doc_path: Option<&Path>) -> PathBuf {
    let stem = doc_path.and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
    dir.join(format!("{stem}.autosave.3dm"))
}

/// Autosave files in `dir` last written more than `older_than` before `now`, newest first.
/// Younger files may belong to another Triceratops window that is still running.
pub fn leftovers(dir: &Path, older_than: Duration, now: SystemTime) -> Vec<(PathBuf, SystemTime)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(PathBuf, SystemTime)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".autosave.3dm")))
        .filter_map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            (now.duration_since(t).unwrap_or_default() >= older_than).then_some((p, t))
        })
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.1));
    out
}

/// "12 minutes ago" for the recovery list.
pub fn ago(t: SystemTime, now: SystemTime) -> String {
    let s = now.duration_since(t).unwrap_or_default().as_secs();
    match s {
        0..=89 => "a minute ago".into(),
        90..=5399 => format!("{} minutes ago", (s + 30) / 60),
        5400..=129_599 => format!("{} hours ago", (s + 1800) / 3600),
        _ => format!("{} days ago", (s + 43_200) / 86_400),
    }
}

/// A snapshot to write.
pub struct SaveJob {
    pub doc: Document,
    pub path: PathBuf,
}

impl SaveJob {
    /// Snapshot of `doc` (its objects, layers and settings) for `path`.
    pub fn of(doc: &Document, path: PathBuf) -> Self {
        let mut copy = Document::new();
        copy.load_state(doc.state.clone(), doc.settings.clone(), None);
        SaveJob { doc: copy, path }
    }
}

/// Write a snapshot now, on this thread.
pub fn write(job: &SaveJob) -> Result<PathBuf, String> {
    if let Some(d) = job.path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    // write next to it, then rename: a crash mid-write never leaves a broken autosave
    let tmp = job.path.with_extension("3dm.part");
    tri_doc::io3dm::save(&job.doc, &tmp, false).map_err(|e| format!("{e:#}"))?;
    std::fs::rename(&tmp, &job.path).map_err(|e| e.to_string())?;
    Ok(job.path.clone())
}

/// When to autosave, and the write in flight.
pub struct Autosaver {
    last_revision: u64,
    last_time: Instant,
    writing: Option<Receiver<Result<PathBuf, String>>>,
    /// The last write failed: say so once, not at every interval.
    failed: bool,
}

impl Autosaver {
    pub fn new(now: Instant, revision: u64) -> Self {
        Autosaver { last_revision: revision, last_time: now, writing: None, failed: false }
    }

    /// Whether a snapshot is due: enabled, `minutes` elapsed since the last one, the model
    /// modified and changed since then, and no write still running.
    pub fn due(&self, now: Instant, doc: &Document, minutes: u32, enabled: bool) -> bool {
        enabled
            && self.writing.is_none()
            && doc.modified
            && doc.revision != self.last_revision
            && now.duration_since(self.last_time) >= Duration::from_secs(u64::from(minutes.max(1)) * 60)
    }

    /// A write is in flight.
    #[cfg(test)]
    pub fn busy(&self) -> bool {
        self.writing.is_some()
    }

    /// Start writing `job` on a background thread.
    pub fn start(&mut self, now: Instant, revision: u64, job: SaveJob) {
        self.last_time = now;
        self.last_revision = revision;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(write(&job));
        });
        self.writing = Some(rx);
    }

    /// The result of a finished write, worth a log line (the first failure, or success
    /// after failures); `None` while writing or when there is nothing to say.
    pub fn poll(&mut self) -> Option<String> {
        let r = self.writing.as_ref()?.try_recv().ok()?;
        self.writing = None;
        match r {
            Ok(p) => {
                let was = std::mem::replace(&mut self.failed, false);
                was.then(|| format!("Autosave written to {}", p.display()))
            }
            Err(e) => (!std::mem::replace(&mut self.failed, true)).then(|| format!("Autosave failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use tri_doc::Geometry;

    #[test]
    fn path_uses_the_file_stem() {
        let d = Path::new("/cfg/autosave");
        assert_eq!(path_in(d, Some(Path::new("/x/plate.3dm"))), PathBuf::from("/cfg/autosave/plate.autosave.3dm"));
        assert_eq!(path_in(d, None), PathBuf::from("/cfg/autosave/Untitled.autosave.3dm"));
    }

    #[test]
    fn due_waits_for_the_interval_and_a_change() {
        let t0 = Instant::now();
        let mut doc = Document::new();
        let a = Autosaver::new(t0, doc.revision);
        let later = t0 + Duration::from_secs(5 * 60);
        assert!(!a.due(later, &doc, 5, true), "unchanged model");
        doc.add(Geometry::Point(DVec3::ZERO));
        assert!(!a.due(t0 + Duration::from_secs(60), &doc, 5, true), "too early");
        assert!(!a.due(later, &doc, 5, false), "disabled");
        assert!(a.due(later, &doc, 5, true));
        doc.modified = false;
        assert!(!a.due(later, &doc, 5, true), "saved since");
    }

    #[test]
    fn writes_a_snapshot_and_reports_failures_once() {
        let dir = std::env::temp_dir().join(format!("tri-autosave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut doc = Document::new();
        doc.add(Geometry::Point(DVec3::new(1.0, 2.0, 3.0)));
        let path = path_in(&dir, None);
        write(&SaveJob::of(&doc, path.clone())).unwrap();
        let back = tri_doc::io3dm::load(&path).unwrap();
        assert_eq!(back.0.objects.len(), 1);
        // a folder that cannot be created: the first failure is reported, the next is not
        let blocked = dir.join("file-not-dir");
        std::fs::write(&blocked, b"x").unwrap();
        let mut a = Autosaver::new(Instant::now(), 0);
        let mut messages = Vec::new();
        for _ in 0..2 {
            a.start(Instant::now(), 1, SaveJob::of(&doc, blocked.join("sub").join("m.autosave.3dm")));
            loop {
                if a.writing.is_none() {
                    break;
                }
                if let Some(m) = a.poll() {
                    messages.push(m);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(messages[0].starts_with("Autosave failed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leftovers_skip_recent_files() {
        let dir = std::env::temp_dir().join(format!("tri-leftovers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.autosave.3dm"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let now = SystemTime::now();
        assert!(leftovers(&dir, Duration::from_secs(600), now).is_empty(), "too recent");
        let later = now + Duration::from_secs(3600);
        let found = leftovers(&dir, Duration::from_secs(600), later);
        assert_eq!(found.len(), 1);
        assert!(found[0].0.ends_with("a.autosave.3dm"));
        assert_eq!(ago(now, later), "60 minutes ago");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
