use crate::packages::valid_name;
use crate::protocol::{self, Message};
use std::fs::{self, File};
use std::io::{self, Read};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

const WORKER: &str = include_str!("../r/worker.R");
const START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum Event {
    Starting,
    Ready { executable: String, version: String },
    Output(String),
    Done(u64),
    PackageStatus(u64, String),
    Environment(u64, String),
    Plot { page: u64, png: Vec<u8> },
    PlotError(String),
    Incomplete(u64, String),
    Error(u64, String),
    Failed(String),
    Exited,
}
#[derive(Debug)]
enum Request {
    Command(u64, &'static str, String),
    Stop,
}

pub struct Session {
    commands: SyncSender<Request>,
    events: Option<Receiver<Event>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Session {
    pub fn start(executable: String, directory: PathBuf) -> Self {
        let (commands, requests) = mpsc::sync_channel(4);
        let (events, rx) = mpsc::sync_channel(256);
        let worker = thread::spawn(move || {
            let result = run(executable, directory, &requests, &events);
            if let Err(error) = result {
                let _ = events.send(Event::Failed(error.to_string()));
            }
            let _ = events.send(Event::Exited);
        });
        Self {
            commands,
            events: Some(rx),
            worker: Some(worker),
        }
    }
    pub fn evaluate(&self, id: u64, text: String) -> Result<(), String> {
        if text.len() > protocol::MAX_FRAME - 64 || text.contains('\0') {
            return Err("Input exceeds the 1 MiB limit or contains a NUL character.".into());
        }
        self.commands
            .try_send(Request::Command(id, "EVAL", text))
            .map_err(|e| format!("Cannot submit command: {e}"))
    }
    pub fn package_status(&self, id: u64, names: &[String]) -> Result<(), String> {
        if names.is_empty() || names.len() > 200 || names.iter().any(|name| !valid_name(name)) {
            return Err("Invalid package list".into());
        }
        self.send_package(id, "PACKAGES", names.join("\n"))
    }
    pub fn environment(&self, id: u64) -> Result<(), String> {
        self.send_package(id, "ENVIRONMENT", String::new())
    }
    pub fn install_package(&self, id: u64, name: &str, bioconductor: bool) -> Result<(), String> {
        if !valid_name(name) {
            return Err("Invalid package name".into());
        }
        self.send_package(
            id,
            "INSTALL",
            format!("{}\t{name}", if bioconductor { "BIOC" } else { "CRAN" }),
        )
    }
    pub fn load_package(&self, id: u64, name: &str) -> Result<(), String> {
        if !valid_name(name) {
            return Err("Invalid package name".into());
        }
        self.send_package(id, "LOAD", name.to_owned())
    }
    fn send_package(&self, id: u64, kind: &'static str, payload: String) -> Result<(), String> {
        self.commands
            .try_send(Request::Command(id, kind, payload))
            .map_err(|e| format!("Cannot send package request: {e}"))
    }
    pub fn poll(&self) -> Vec<Event> {
        self.events
            .as_ref()
            .map(|rx| rx.try_iter().take(256).collect())
            .unwrap_or_default()
    }
    pub fn stop(&self) {
        let _ = self.commands.try_send(Request::Stop);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        // Disconnect before joining so a full event queue cannot block shutdown.
        self.events.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn random_secret() -> io::Result<String> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn stopped(rx: &Receiver<Request>) -> bool {
    matches!(
        rx.try_recv(),
        Ok(Request::Stop) | Err(TryRecvError::Disconnected)
    )
}

fn run(
    executable: String,
    directory: PathBuf,
    requests: &Receiver<Request>,
    events: &SyncSender<Event>,
) -> io::Result<()> {
    let _ = events.send(Event::Starting);
    if !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Working directory does not exist",
        ));
    }
    let executable = if executable.trim().is_empty() {
        "R".to_owned()
    } else {
        executable.trim().to_owned()
    };
    let executable = resolve_executable(&executable)?;
    let version = version(&executable, requests)?;
    let secret = random_secret()?;
    let scratch_name = random_secret()?;
    let scratch = Scratch(
        std::env::temp_dir().join(format!("rsstudio-{}-{scratch_name}", std::process::id())),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&scratch.0)?;
    }
    #[cfg(not(unix))]
    fs::create_dir(&scratch.0)?;
    let script = scratch.0.join("worker.R");
    fs::write(&script, WORKER)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let mut cmd = Command::new(&executable);
    cmd.args(["--vanilla", "--quiet", "--no-echo", "-f"])
        .arg(script)
        .env("RSSTUDIO_PORT", port.to_string())
        .env("RSSTUDIO_SECRET", &secret)
        .env("RSSTUDIO_PLOT_DIR", &scratch.0)
        .env("LC_MESSAGES", "C")
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("Could not start R '{executable}': {e}")))?;
    stream_output(child.stdout.take().unwrap(), events.clone(), "");
    stream_output(child.stderr.take().unwrap(), events.clone(), "[stderr] ");
    let result = connected_loop(
        &listener,
        &secret,
        (&executable, &version),
        &mut child,
        requests,
        events,
        &scratch.0,
    );
    terminate(&mut child);
    result
}

fn resolve_executable(name: &str) -> io::Result<String> {
    let path = std::path::Path::new(name);
    let candidate = if path.components().count() > 1 || path.is_absolute() {
        Some(if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        })
    } else {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(path))
                .find(|file| file.is_file())
        })
    };
    let candidate = candidate.filter(|p| p.is_file()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("R executable '{name}' was not found. Install R or enter its full path."),
        )
    })?;
    Ok(candidate.to_string_lossy().into_owned())
}

fn version(executable: &str, requests: &Receiver<Request>) -> io::Result<String> {
    let mut child = Command::new(executable).arg("--version").stdin(Stdio::null())
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("R executable '{executable}' could not be launched: {e}. Install R or enter its full path.")))?;
    let stdout = child.stdout.take().unwrap();
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(64 * 1024).read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    loop {
        if stopped(requests) || started.elapsed() > START_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "R version check cancelled or timed out",
            ));
        }
        if let Some(status) = child.try_wait()? {
            let output = reader.join().unwrap_or_default();
            let text = String::from_utf8_lossy(&output);
            let first = text.lines().next().unwrap_or_default().trim();
            if !status.success() || !first.starts_with("R version ") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "The selected executable did not report a valid R version",
                ));
            }
            return Ok(first.to_owned());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn connected_loop(
    listener: &TcpListener,
    secret: &str,
    runtime: (&str, &str),
    child: &mut Child,
    requests: &Receiver<Request>,
    events: &SyncSender<Event>,
    plot_dir: &std::path::Path,
) -> io::Result<()> {
    let started = Instant::now();
    let stream = loop {
        if stopped(requests) {
            return Ok(());
        }
        if child.try_wait()?.is_some() {
            return Err(io::Error::other(
                "R exited during startup; check the console output",
            ));
        }
        if started.elapsed() > START_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "R did not connect within 10 seconds",
            ));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_millis(500)))?;
                let hello = protocol::read_message(&mut &stream);
                if let Ok(Message {
                    kind,
                    id: 0,
                    payload,
                }) = hello
                    && kind == "HELLO"
                    && payload == format!("1:{secret}")
                {
                    break stream;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(e) => return Err(e),
        }
    };
    stream.set_read_timeout(None)?;
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(Some(Duration::from_millis(500)))?;
    let (tx, rx) = mpsc::sync_channel(4);
    thread::spawn(move || {
        let mut reader = stream;
        loop {
            let message = protocol::read_message(&mut reader);
            let failed = message.is_err();
            if tx.send(message).is_err() || failed {
                break;
            }
        }
    });
    let _ = events.send(Event::Ready {
        executable: runtime.0.into(),
        version: runtime.1.into(),
    });
    let mut pending = None;
    loop {
        if child.try_wait()?.is_some() {
            return Err(io::Error::other("R exited unexpectedly"));
        }
        match requests.try_recv() {
            Ok(Request::Stop) | Err(TryRecvError::Disconnected) => {
                if pending.is_none() {
                    let _ = protocol::write_message(&mut writer, "SHUTDOWN", 0, "");
                    let deadline = Instant::now() + Duration::from_millis(500);
                    while Instant::now() < deadline {
                        if child.try_wait()?.is_some() {
                            return Ok(());
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                }
                return Ok(());
            }
            Ok(Request::Command(id, kind, text)) => {
                if pending.is_some() {
                    let _ = events.send(Event::Error(id, "R is already busy".into()));
                } else {
                    protocol::write_message(&mut writer, kind, id, &text)?;
                    pending = Some((id, kind));
                }
            }
            Err(TryRecvError::Empty) => {}
        }
        match rx.try_recv() {
            Ok(Ok(msg)) => {
                if pending.map(|(id, _)| id) != Some(msg.id) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Unexpected R response ID",
                    ));
                }
                if msg.kind == "PLOT" && pending == Some((msg.id, "EVAL")) {
                    let event = match read_plot(plot_dir, &msg.payload) {
                        Ok((page, png)) => Event::Plot { page, png },
                        Err(error) => Event::PlotError(error.to_string()),
                    };
                    let _ = events.send(event);
                    continue;
                }
                let event = match msg.kind.as_str() {
                    "STATUS" if pending == Some((msg.id, "PACKAGES")) => {
                        Event::PackageStatus(msg.id, msg.payload)
                    }
                    "ENVIRONMENT" if pending == Some((msg.id, "ENVIRONMENT")) => {
                        Event::Environment(msg.id, msg.payload)
                    }
                    "DONE" if pending != Some((msg.id, "PACKAGES")) => Event::Done(msg.id),
                    "INCOMPLETE" if pending == Some((msg.id, "EVAL")) => {
                        Event::Incomplete(msg.id, msg.payload)
                    }
                    "ERROR" => Event::Error(msg.id, msg.payload),
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Unexpected R response",
                        ));
                    }
                };
                pending = None;
                let _ = events.send(event);
            }
            Ok(Err(e)) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("R control connection closed: {e}"),
                ));
            }
            Err(TryRecvError::Disconnected) => {
                return Err(io::Error::other("R control reader stopped"));
            }
            Err(TryRecvError::Empty) => {}
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn read_plot(directory: &std::path::Path, payload: &str) -> io::Result<(u64, Vec<u8>)> {
    let (page, filename) = payload
        .split_once('\t')
        .ok_or_else(|| io::Error::other("Invalid plot message"))?;
    let page = page
        .parse::<u64>()
        .map_err(|_| io::Error::other("Invalid plot page"))?;
    let sequence = filename
        .strip_prefix("plot-")
        .and_then(|v| v.strip_suffix(".png"))
        .filter(|v| !v.is_empty() && v.len() <= 16 && v.bytes().all(|c| c.is_ascii_digit()))
        .ok_or_else(|| io::Error::other("Invalid plot filename"))?;
    let _ = sequence;
    let path = directory.join(filename);
    let file = File::open(&path).map_err(|error| {
        io::Error::new(error.kind(), format!("Cannot read {filename}: {error}"))
    })?;
    let mut png = Vec::new();
    file.take(8 * 1024 * 1024 + 1).read_to_end(&mut png)?;
    let _ = fs::remove_file(path);
    if png.len() > 8 * 1024 * 1024 || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(io::Error::other("Invalid or oversized plot image"));
    }
    Ok((page, png))
}

fn stream_output(
    mut stream: impl Read + Send + 'static,
    events: SyncSender<Event>,
    prefix: &'static str,
) {
    thread::spawn(move || {
        let mut buf = [0; 4096];
        let mut pending = Vec::new();
        let mut dropped = false;
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            pending.extend_from_slice(&buf[..n]);
            let valid = match std::str::from_utf8(&pending) {
                Ok(_) => pending.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(_) => pending.len(),
            };
            let text = String::from_utf8_lossy(&pending[..valid]);
            let marker = if dropped {
                "[output truncated: interface could not keep up]\n"
            } else {
                ""
            };
            if !text.is_empty() {
                dropped = events
                    .try_send(Event::Output(format!("{marker}{prefix}{text}")))
                    .is_err();
            }
            pending.drain(..valid);
        }
        if dropped {
            let _ = events.send(Event::Output(
                "[output truncated: interface could not keep up]\n".into(),
            ));
        }
        if !pending.is_empty() {
            let _ = events.try_send(Event::Output(format!(
                "{prefix}{}",
                String::from_utf8_lossy(&pending)
            )));
        }
    });
}

fn terminate(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        #[cfg(unix)]
        {
            // The child is its own process group, so descendants are stopped as well.
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            unsafe {
                kill(-(child.id() as i32), 9);
            }
        }
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[cfg(test)]
mod plot_tests {
    use super::*;
    #[test]
    fn plot_messages_cannot_escape_the_session_directory() {
        let dir =
            std::env::temp_dir().join(format!("rsstudio-plot-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = b"\x89PNG\r\n\x1a\ntest";
        std::fs::write(dir.join("plot-0001.png"), bytes).unwrap();
        assert!(read_plot(&dir, "1\t../plot-0001.png").is_err());
        assert!(read_plot(&dir, "1\tplot-x.png").is_err());
        assert_eq!(
            read_plot(&dir, "7\tplot-0001.png").unwrap(),
            (7, bytes.to_vec())
        );
        assert!(!dir.join("plot-0001.png").exists());
        std::fs::remove_dir(dir).unwrap();
    }
}
