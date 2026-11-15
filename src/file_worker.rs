//! Bounded disk work queue. UI code only submits snapshots and polls results.
use crate::{editor::Buffer, settings::Settings};
use std::{
    fs, io,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
};

pub type Entries = Vec<(PathBuf, bool)>;
pub enum Task {
    Open(PathBuf),
    Save(Buffer, PathBuf),
    List(PathBuf),
    Preferences(Settings),
    ExportPlot(PathBuf, Vec<u8>),
}
pub enum Event {
    Open(io::Result<Buffer>),
    Save(io::Result<Buffer>),
    List(io::Result<(PathBuf, Entries)>),
    Preferences(io::Result<()>),
    ExportPlot(io::Result<()>),
}
pub struct Worker {
    commands: SyncSender<Task>,
    events: Receiver<Event>,
}
impl Worker {
    pub fn new() -> Self {
        let (commands, requests) = mpsc::sync_channel(4);
        let (events, responses) = mpsc::sync_channel(4);
        std::thread::spawn(move || {
            while let Ok(task) = requests.recv() {
                let event = perform(task);
                if events.send(event).is_err() {
                    break;
                }
            }
        });
        Self {
            commands,
            events: responses,
        }
    }
    pub fn submit(&self, task: Task) -> Result<(), String> {
        self.commands
            .try_send(task)
            .map_err(|e| format!("Cannot queue file operation: {e}"))
    }
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().take(4).collect()
    }
}
fn perform(task: Task) -> Event {
    match task {
        Task::Open(path) => Event::Open(Buffer::read(path)),
        Task::Save(mut buffer, path) => Event::Save(buffer.save(&path).map(|()| buffer)),
        Task::ExportPlot(path, png) => {
            Event::ExportPlot(crate::plots::Plots::write_export(&path, &png))
        }
        Task::Preferences(settings) => Event::Preferences(settings.save()),
        Task::List(path) => Event::List((|| {
            let path = fs::canonicalize(path)?;
            let mut entries = fs::read_dir(&path)?
                .take(10_000)
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let kind = entry.file_type().ok()?;
                    (kind.is_file() || kind.is_dir()).then(|| (entry.path(), kind.is_dir()))
                })
                .collect::<Entries>();
            entries.sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));
            Ok((path, entries))
        })()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use std::time::{Duration, Instant};

    fn next(worker: &Worker) -> Event {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(event) = worker.poll().into_iter().next() {
                return event;
            }
            assert!(Instant::now() < deadline, "Disk worker did not complete");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn worker_recovers_from_errors_and_returns_saved_snapshots() {
        let dir = std::env::temp_dir().join(format!("rsstudio-worker-test-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("script.R");
        let worker = Worker::new();
        worker.submit(Task::Open(path.clone())).unwrap();
        assert!(matches!(next(&worker), Event::Open(Err(_))));
        let mut editor = Editor::new();
        editor.current_mut().text = "x <- 1\n".into();
        worker
            .submit(Task::Save(editor.current().clone(), path.clone()))
            .unwrap();
        editor.current_mut().text = "newer edits".into();
        let Event::Save(result) = next(&worker) else {
            panic!("Expected saved snapshot")
        };
        editor.accept_save(result.unwrap());
        assert!(editor.current().dirty());
        assert_eq!(editor.current().text, "newer edits");
        worker.submit(Task::Open(path.clone())).unwrap();
        let Event::Open(result) = next(&worker) else {
            panic!("Expected opened file")
        };
        assert_eq!(result.unwrap().text, "x <- 1\n");
        worker.submit(Task::List(dir.clone())).unwrap();
        let Event::List(result) = next(&worker) else {
            panic!("Expected directory listing")
        };
        let (canonical, entries) = result.unwrap();
        assert_eq!(canonical, fs::canonicalize(&dir).unwrap());
        assert_eq!(entries, vec![(path, false)]);
        fs::remove_dir_all(dir).unwrap();
    }
}
