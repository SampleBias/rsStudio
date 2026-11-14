//! Script buffers, bounded file I/O, and selection handling.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_FILE: u64 = 2 * 1024 * 1024;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct Buffer {
    pub id: u64,
    pub path: Option<PathBuf>,
    pub text: String,
    saved: String,
    /// Exact bytes last read or written, for external-change detection.
    disk: Option<Vec<u8>>,
    pub cursor: (usize, usize), // character offsets, not byte offsets
}

impl Buffer {
    fn untitled(id: u64) -> Self {
        Self {
            id,
            path: None,
            text: String::new(),
            saved: String::new(),
            disk: None,
            cursor: (0, 0),
        }
    }
    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("Untitled {}", self.id))
    }
    pub fn dirty(&self) -> bool {
        self.text != self.saved
    }
    pub fn selected_or_line(&self) -> &str {
        let (a, b) = self.cursor;
        let start = char_byte(&self.text, a.min(b));
        let end = char_byte(&self.text, a.max(b));
        if start != end {
            return &self.text[start..end];
        }
        let line_start = self.text[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = self.text[start..]
            .find('\n')
            .map_or(self.text.len(), |i| start + i);
        &self.text[line_start..line_end]
    }
    pub fn save(&mut self, destination: &Path) -> io::Result<()> {
        if self.text.len() as u64 > MAX_FILE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Script exceeds the 2 MiB editor limit",
            ));
        }
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let destination = fs::canonicalize(parent)?.join(
            destination
                .file_name()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Enter a file name"))?,
        );
        let destination = destination.as_path();
        let same_file = self.path.as_deref() == Some(destination);
        let expected = if same_file {
            self.disk.as_deref()
        } else {
            None
        };
        save_file(destination, self.text.as_bytes(), expected)?;
        self.path = Some(destination.to_owned());
        self.saved.clone_from(&self.text);
        self.disk = Some(self.text.as_bytes().to_vec());
        Ok(())
    }
}

pub struct Editor {
    pub buffers: Vec<Buffer>,
    pub active: usize,
    next_id: u64,
}
impl Editor {
    pub fn new() -> Self {
        Self {
            buffers: vec![Buffer::untitled(1)],
            active: 0,
            next_id: 2,
        }
    }
    pub fn current(&self) -> &Buffer {
        &self.buffers[self.active]
    }
    pub fn current_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.active]
    }
    pub fn new_buffer(&mut self) {
        self.buffers.push(Buffer::untitled(self.next_id));
        self.next_id += 1;
        self.active = self.buffers.len() - 1;
    }
    pub fn open(&mut self, path: PathBuf) -> io::Result<()> {
        let path = fs::canonicalize(path)?;
        if let Some(index) = self
            .buffers
            .iter()
            .position(|buffer| buffer.path.as_ref() == Some(&path))
        {
            self.active = index;
            return Ok(());
        }
        let bytes = read_file(&path)?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "File is not valid UTF-8"))?;
        self.buffers.push(Buffer {
            id: self.next_id,
            path: Some(path),
            saved: text.clone(),
            text,
            disk: Some(bytes),
            cursor: (0, 0),
        });
        self.next_id += 1;
        self.active = self.buffers.len() - 1;
        Ok(())
    }
    pub fn close(&mut self, index: usize) {
        self.buffers.remove(index);
        if self.buffers.is_empty() {
            self.new_buffer();
        } else if index < self.active {
            self.active -= 1;
        } else {
            self.active = self.active.min(self.buffers.len() - 1);
        }
    }
    pub fn any_dirty(&self) -> bool {
        self.buffers.iter().any(Buffer::dirty)
    }
}

fn char_byte(text: &str, pos: usize) -> usize {
    text.char_indices().nth(pos).map_or(text.len(), |(i, _)| i)
}

fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "File exceeds the 2 MiB editor limit",
        ));
    }
    Ok(bytes)
}

fn save_file(path: &Path, bytes: &[u8], expected: Option<&[u8]>) -> io::Result<()> {
    // Refuse links and unexpected existing files, including a Save As collision.
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() || expected.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Destination exists or is not a regular file; choose a new path",
                ));
            }
            if read_file(path)? != expected.unwrap() {
                return Err(io::Error::other(
                    "File changed on disk; use Save As to preserve both versions",
                ));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound && expected.is_none() => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::other(
                "File was removed from disk; use Save As to preserve your work",
            ));
        }
        Err(e) => return Err(e),
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Enter a file name"))?;
    let mut temp_name = name.to_os_string();
    temp_name.push(format!(
        ".rsstudio-{}-{}.tmp",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let temp = parent.join(temp_name);
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        if expected.is_some() {
            file.set_permissions(fs::metadata(path)?.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Some(expected) = expected {
            if !fs::symlink_metadata(path)?.file_type().is_file() || read_file(path)? != expected {
                return Err(io::Error::other(
                    "File changed on disk during save; use Save As",
                ));
            }
            fs::rename(&temp, path)?;
        } else {
            // hard_link is atomic and fails if the destination appeared meanwhile.
            fs::hard_link(&temp, path)?;
            // The destination is already safely created; a cleanup failure must
            // not report the successful save as a failure.
            let _ = fs::remove_file(&temp);
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tmp() -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "rsstudio-editor-test-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        TempDir(path)
    }
    #[test]
    fn saves_and_refuses_external_changes_and_collisions() {
        let dir = tmp();
        let path = dir.path().join("test.R");
        let mut editor = Editor::new();
        editor.current_mut().text = "x <- 1\n".into();
        editor.current_mut().save(&path).unwrap();
        assert!(!editor.current().dirty());
        editor.current_mut().text = "x <- 10\n".into();
        editor.current_mut().save(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "x <- 10\n");
        fs::write(&path, "external").unwrap();
        editor.current_mut().text = "x <- 2".into();
        assert!(editor.current_mut().save(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        let mut other = Editor::new();
        assert!(other.current_mut().save(&path).is_err());
        let alternate = dir.path().join("other.R");
        editor.current_mut().save(&alternate).unwrap();
        assert_eq!(fs::read_to_string(alternate).unwrap(), "x <- 2");
    }
    #[test]
    fn utf8_selection_and_current_line() {
        let mut b = Buffer::untitled(1);
        b.text = "α <- 1\nx <- α\n".into();
        b.cursor = (7, 7);
        assert_eq!(b.selected_or_line(), "x <- α");
        b.cursor = (1, 0);
        assert_eq!(b.selected_or_line(), "α");
    }
    #[test]
    fn refuses_invalid_utf8_and_limits_files() {
        let dir = tmp();
        let path = dir.path().join("bad.R");
        fs::write(&path, [0xff]).unwrap();
        assert!(Editor::new().open(path.clone()).is_err());
        fs::write(&path, vec![b'x'; MAX_FILE as usize + 1]).unwrap();
        assert!(Editor::new().open(path).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn save_as_does_not_follow_symlink() {
        let dir = tmp();
        let target = dir.path().join("target.R");
        let link = dir.path().join("link.R");
        fs::write(&target, "original").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut editor = Editor::new();
        editor.current_mut().text = "new".into();
        assert!(editor.current_mut().save(&link).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "original");
    }
    #[test]
    fn opens_existing_file_only_once_and_keeps_dirty_text() {
        let dir = tmp();
        let path = dir.path().join("script.R");
        fs::write(&path, "x <- 1").unwrap();
        let mut editor = Editor::new();
        editor.open(path.clone()).unwrap();
        editor.current_mut().text = "unsaved".into();
        editor.open(path).unwrap();
        assert_eq!(editor.buffers.len(), 2);
        assert_eq!(editor.current().text, "unsaved");
        assert!(editor.any_dirty());
    }
}
