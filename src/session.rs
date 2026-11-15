use crate::packages::valid_name;
use crate::protocol;
use std::fs::{self, File};
use std::io::{self, Read};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
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
    Completions(u64, String),
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
    control: Arc<Control>,
}
impl Session {
    pub fn start(executable: String, directory: PathBuf) -> Self {
        let (commands, requests) = mpsc::sync_channel(4);
        let (events, rx) = mpsc::sync_channel(256);
        let control = Arc::new(Control::default());
        let worker_control = Arc::clone(&control);
        thread::spawn(move || {
            let result = run(executable, directory, &requests, &events, &worker_control);
            if let Err(error) = result {
                let _ = events.send(Event::Failed(error.to_string()));
            }
            let _ = events.send(Event::Exited);
        });
        Self {
            commands,
            events: Some(rx),
            control,
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
    pub fn complete(&self, id: u64, payload: String) -> Result<(), String> {
        if payload.len() > 10_000 || payload.contains('\0') {
            return Err("Completion request is too large.".into());
        }
        self.send_package(id, "COMPLETE", payload)
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
        self.control.cancelled.store(true, Ordering::Release);
        let _ = self.commands.try_send(Request::Stop);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        // Cancellation and group termination do not wait for pipes or filesystem I/O.
        // The supervisor reaps the child and removes its scratch directory off the UI.
        self.events.take();
        self.control.kill_group();
    }
}

#[derive(Default)]
struct Control {
    cancelled: AtomicBool,
    group: AtomicU32,
}
impl Control {
    fn register(&self, child: &Child) {
        self.group.store(child.id(), Ordering::Release);
        if self.cancelled.load(Ordering::Acquire) {
            self.kill_group();
        }
    }
    fn kill_group(&self) {
        let group = self.group.swap(0, Ordering::AcqRel);
        #[cfg(unix)]
        if group != 0 {
            // All supervised processes start in an owned group, including --version.
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            unsafe {
                kill(-(group as i32), 9);
            }
        }
        #[cfg(not(unix))]
        let _ = group;
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
    control: &Control,
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
    let version = version(&executable, requests, control)?;
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
    fs::write(&script, worker_source())?;
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
    let mut child = spawn_before(&mut cmd, control, Instant::now() + START_TIMEOUT)
        .map_err(|e| io::Error::new(e.kind(), format!("Could not start R '{executable}': {e}")))?;
    control.register(&child);
    stream_output(child.stdout.take().unwrap(), events.clone(), "");
    stream_output(child.stderr.take().unwrap(), events.clone(), "[stderr] ");
    let result = connected_loop(
        &listener,
        &secret,
        (&executable, &version),
        &mut child,
        requests,
        events,
        (control, &scratch.0),
    );
    terminate(&mut child, control);
    result
}

pub(crate) fn worker_source() -> String {
    // Keep helper definitions out of the user's .GlobalEnv without relying on
    // textual insertion into the worker's body.
    format!(
        "local({{\n{}\n{}\n}})\n",
        include_str!("../r/warnings.R"),
        WORKER
    )
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

fn spawn_before(command: &mut Command, control: &Control, deadline: Instant) -> io::Result<Child> {
    loop {
        if control.cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "R launch cancelled or timed out",
            ));
        }
        match command.spawn() {
            // Wrappers and executables can briefly remain open for writing during
            // installation or filesystem synchronization. Retry within the deadline.
            Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy => {
                thread::sleep(Duration::from_millis(10))
            }
            result => return result,
        }
    }
}

fn version(
    executable: &str,
    requests: &Receiver<Request>,
    control: &Control,
) -> io::Result<String> {
    version_with_timeout(executable, requests, control, START_TIMEOUT)
}

fn version_with_timeout(
    executable: &str,
    requests: &Receiver<Request>,
    control: &Control,
    timeout: Duration,
) -> io::Result<String> {
    let deadline = Instant::now() + timeout;
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = spawn_before(&mut command, control, deadline).map_err(|e| io::Error::new(e.kind(), format!("R executable '{executable}' could not be launched: {e}. Install R or enter its full path.")))?;
    control.register(&child);
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout
            .take(64 * 1024)
            .read_to_end(&mut output)
            .map(|_| output);
        let _ = tx.send(result);
    });
    let result = (|| {
        let mut status = None;
        let mut output = None;
        loop {
            if control.cancelled.load(Ordering::Acquire)
                || stopped(requests)
                || Instant::now() >= deadline
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "R version check cancelled or timed out",
                ));
            }
            if status.is_none() {
                status = child.try_wait()?;
                if status.is_some() {
                    // A wrapper may exit while a descendant retains the stdout pipe.
                    control.kill_group();
                }
            }
            if output.is_none() {
                match rx.try_recv() {
                    Ok(result) => output = Some(result?),
                    Err(TryRecvError::Disconnected) => {
                        return Err(io::Error::other("R version output reader stopped"));
                    }
                    Err(TryRecvError::Empty) => {}
                }
            }
            if let (Some(status), Some(output)) = (&status, &output) {
                let text = String::from_utf8_lossy(output);
                let first = text.lines().next().unwrap_or_default().trim();
                if !status.success() || !first.starts_with("R version ") {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "The selected executable did not report a valid R version",
                    ));
                }
                return Ok(first.to_owned());
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    terminate(&mut child, control);
    result
}

fn read_hello(
    mut reader: impl Read,
    secret: &str,
    deadline: Instant,
    control: &Control,
) -> io::Result<()> {
    // A known-size greeting prevents a partial or oversized unauthenticated frame
    // from extending startup indefinitely. The socket is nonblocking here.
    let mut expected = Vec::new();
    protocol::write_message(&mut expected, "HELLO", 0, &format!("1:{secret}"))?;
    let mut received = vec![0; expected.len()];
    let mut offset = 0;
    while offset < received.len() {
        if control.cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "R greeting cancelled or timed out",
            ));
        }
        match reader.read(&mut received[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Incomplete R greeting",
                ));
            }
            Ok(n) => {
                let end = offset + n;
                if received[offset..end] != expected[offset..end] {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid R greeting",
                    ));
                }
                offset = end;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn connected_loop(
    listener: &TcpListener,
    secret: &str,
    runtime: (&str, &str),
    child: &mut Child,
    requests: &Receiver<Request>,
    events: &SyncSender<Event>,
    resources: (&Control, &std::path::Path),
) -> io::Result<()> {
    let (control, plot_dir) = resources;
    let started = Instant::now();
    let stream = loop {
        if control.cancelled.load(Ordering::Acquire) || stopped(requests) {
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
            Ok((mut stream, _)) => {
                stream.set_nonblocking(true)?;
                let deadline =
                    (Instant::now() + Duration::from_millis(500)).min(started + START_TIMEOUT);
                if read_hello(&mut stream, secret, deadline, control).is_ok() {
                    stream.set_nonblocking(false)?;
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
        let request = if control.cancelled.load(Ordering::Acquire) {
            Ok(Request::Stop)
        } else if matches!(pending, Some((_, "COMPLETE"))) {
            // Keep foreground work queued until the current lookup finishes.
            Err(TryRecvError::Empty)
        } else {
            requests.try_recv()
        };
        match request {
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
                    "COMPLETIONS" if pending == Some((msg.id, "COMPLETE")) => {
                        Event::Completions(msg.id, msg.payload)
                    }
                    "DONE" if matches!(pending, Some((_, "EVAL" | "INSTALL" | "LOAD"))) => {
                        Event::Done(msg.id)
                    }
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
    if !fs::metadata(&path)?.is_file() {
        return Err(io::Error::other("Plot image is not a regular file"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x800); // O_NONBLOCK prevents a FIFO replacement from hanging cleanup.
    }
    let file = options.open(&path).map_err(|error| {
        io::Error::new(error.kind(), format!("Cannot read {filename}: {error}"))
    })?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Plot image is not a regular file"));
    }
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

fn terminate(child: &mut Child, control: &Control) {
    // Kill the owned group even if its leader has already exited.
    control.kill_group();
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[cfg(all(test, target_os = "linux"))]
mod lifecycle_tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};

    struct Script(PathBuf);
    impl Script {
        fn new(body: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rsstudio-process-test-{}-{}",
                std::process::id(),
                random_secret().unwrap()
            ));
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }
    impl Drop for Script {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    fn running(pid: u32) -> bool {
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| s.rsplit_once(") ").map(|(_, rest)| !rest.starts_with('Z')))
            .unwrap_or(false)
    }
    fn assert_stopped(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while running(pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!running(pid), "descendant {pid} survived cleanup");
    }
    #[test]
    fn partial_greeting_obeys_one_deadline_and_cancellation() {
        struct Stalled;
        impl Read for Stalled {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }
        let control = Control::default();
        let started = Instant::now();
        let error = read_hello(
            Stalled,
            "secret",
            started + Duration::from_millis(60),
            &control,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        control.cancelled.store(true, Ordering::Release);
        assert!(read_hello(Stalled, "secret", Instant::now() + START_TIMEOUT, &control).is_err());
        control.cancelled.store(false, Ordering::Release);
        let mut greeting = Vec::new();
        protocol::write_message(&mut greeting, "HELLO", 0, "1:secret").unwrap();
        read_hello(
            std::io::Cursor::new(&greeting),
            "secret",
            Instant::now() + START_TIMEOUT,
            &control,
        )
        .unwrap();
        assert!(
            read_hello(
                std::io::Cursor::new(greeting),
                "wrong-secret",
                Instant::now() + START_TIMEOUT,
                &control
            )
            .is_err()
        );
    }
    #[test]
    fn version_retries_a_temporarily_busy_executable() {
        let script = Script::new("printf 'R version 4.4.0 (test)\\n'");
        let writer = fs::OpenOptions::new().append(true).open(&script.0).unwrap();
        let path = script.0.clone();
        let (tx, result) = mpsc::channel();
        thread::spawn(move || {
            let (_commands, requests) = mpsc::sync_channel(1);
            let _ = tx.send(version_with_timeout(
                path.to_str().unwrap(),
                &requests,
                &Control::default(),
                Duration::from_secs(2),
            ));
        });
        thread::sleep(Duration::from_millis(80));
        drop(writer);
        assert!(
            result
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap()
                .starts_with("R version ")
        );
    }
    #[test]
    fn version_pipe_inherited_by_descendant_does_not_block() {
        let script = Script::new("sleep 30 &\nprintf 'R version 4.4.0 (test)\\n'\nexit 0");
        let (_tx, rx) = mpsc::sync_channel(1);
        let control = Control::default();
        let started = Instant::now();
        let version = version_with_timeout(
            script.0.to_str().unwrap(),
            &rx,
            &control,
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(version.starts_with("R version "));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn version_deadline_kills_a_hung_wrapper() {
        let script = Script::new("exec sleep 30");
        let (_tx, rx) = mpsc::sync_channel(1);
        let control = Control::default();
        let started = Instant::now();
        let result = version_with_timeout(
            script.0.to_str().unwrap(),
            &rx,
            &control,
            Duration::from_millis(150),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(control.group.load(Ordering::Acquire), 0);
    }
    #[test]
    fn cleanup_kills_descendants_after_group_leader_exits() {
        let control = Control::default();
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & echo $!; exit 0"])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        control.register(&child);
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let descendant = line.trim().parse().unwrap();
        child.wait().unwrap();
        terminate(&mut child, &control);
        assert_stopped(descendant);
    }
    #[test]
    fn dropping_session_cancels_version_check_without_joining_ui() {
        let marker = Script::new("");
        fs::remove_file(&marker.0).unwrap();
        let script = Script::new(&format!(
            "echo $$ > '{}'\nexec sleep 30",
            marker.0.display()
        ));
        let session = Session::start(
            script.0.to_string_lossy().into_owned(),
            std::env::temp_dir(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Some(pid) = fs::read_to_string(&marker.0)
                .ok()
                .and_then(|s| s.trim().parse().ok())
            {
                break pid;
            }
            for event in session.poll() {
                if let Event::Failed(error) = event {
                    panic!("Test wrapper failed to start: {error}");
                }
            }
            assert!(Instant::now() < deadline, "Test wrapper did not start");
            thread::sleep(Duration::from_millis(10));
        };
        let started = Instant::now();
        drop(session);
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_stopped(pid);
    }
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
        #[cfg(target_os = "linux")]
        {
            let fifo = dir.join("plot-0002.png");
            assert!(
                Command::new("mkfifo")
                    .arg(&fifo)
                    .status()
                    .unwrap()
                    .success()
            );
            let read_dir = dir.clone();
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = tx.send(read_plot(&read_dir, "2\tplot-0002.png"));
            });
            assert!(
                rx.recv_timeout(Duration::from_secs(1))
                    .expect("Plot FIFO blocked the supervisor")
                    .is_err()
            );
            fs::remove_file(fifo).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }
}
