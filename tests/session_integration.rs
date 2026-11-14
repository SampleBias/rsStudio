#[path = "../src/protocol.rs"]
mod protocol;
#[path = "../src/session.rs"]
mod session;

use session::{Event, Session};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn wait_for(session: &Session, predicate: impl Fn(&Event) -> bool) -> (Event, String) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = String::new();
    loop {
        let mut found = None;
        for event in session.poll() {
            if let Event::Output(text) = &event {
                output.push_str(text);
            }
            if let Event::Failed(message) = &event {
                panic!("session failed: {message}");
            }
            if found.is_none() && predicate(&event) {
                found = Some(event);
            }
        }
        if let Some(event) = found {
            return (event, output);
        }
        assert!(
            Instant::now() < deadline,
            "session event timed out: {output}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn fake_r() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rsstudio-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fake-r");
    fs::write(
        &path,
        r##"#!/usr/bin/env python3
import os, socket, struct, sys, time
if '--version' in sys.argv:
 print('R version 4.4.0 (fake test runtime)'); sys.exit(0)
s=socket.create_connection(('127.0.0.1',int(os.environ['RSSTUDIO_PORT'])))
def send(kind, ident, body=''):
 b=f'{kind}\t{ident}\t{body}'.encode(); s.sendall(struct.pack('>I',len(b))+b)
def read(n):
 b=b''
 while len(b)<n:
  part=s.recv(n-len(b))
  if not part: return None
  b+=part
 return b
send('HELLO',0,'1:'+os.environ['RSSTUDIO_SECRET'])
x=None
while True:
 h=read(4)
 if h is None: break
 b=read(struct.unpack('>I',h)[0]); kind, ident, value=b.decode().split('\t',2)
 if kind == 'SHUTDOWN': break
 if value == 'x <- 41': x=41
 elif value == 'x + 1': print('[1] '+str(x+1) if x is not None else 'missing',flush=True)
 elif value == 'error': send('ERROR',ident,'fake failure'); continue
 elif value == 'incomplete': send('INCOMPLETE',ident,'unexpected end of input'); continue
 elif value == 'crash': os._exit(1)
 elif value == 'wait': time.sleep(30)
 send('DONE',ident)
"##,
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[test]
fn persistent_session_recovers_and_restarts() {
    if !network_available() {
        return;
    }
    let executable = fake_r();
    let session = Session::start(executable.display().to_string(), std::env::temp_dir());
    let (ready, _) = wait_for(&session, |e| matches!(e, Event::Ready { .. }));
    if let Event::Ready {
        executable: path,
        version,
    } = ready
    {
        assert!(path.ends_with("fake-r"));
        assert!(version.starts_with("R version"));
    }
    session.evaluate(1, "x <- 41".into()).unwrap();
    wait_for(&session, |e| matches!(e, Event::Done(1)));
    session.evaluate(2, "error".into()).unwrap();
    let (event, _) = wait_for(&session, |e| matches!(e, Event::Error(2, _)));
    assert!(matches!(event, Event::Error(_, message) if message.contains("fake failure")));
    session.evaluate(3, "incomplete".into()).unwrap();
    let (event, _) = wait_for(&session, |e| matches!(e, Event::Incomplete(3, _)));
    assert!(matches!(event, Event::Incomplete(_, message) if message.contains("unexpected")));
    session.evaluate(4, "x + 1".into()).unwrap();
    let (_, mut output) = wait_for(&session, |e| matches!(e, Event::Done(4)));
    // stdout and the control socket can arrive in either order.
    let end = Instant::now() + Duration::from_secs(2);
    while !output.contains("42") && Instant::now() < end {
        for event in session.poll() {
            if let Event::Output(text) = event {
                output.push_str(&text);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(output.contains("42"), "{output}");
    session.evaluate(5, "wait".into()).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    drop(session);
    assert!(started.elapsed() < Duration::from_secs(3));
    let next = Session::start(executable.display().to_string(), std::env::temp_dir());
    wait_for(&next, |e| matches!(e, Event::Ready { .. }));
    drop(next);
}

#[test]
fn unexpected_r_exit_is_reported() {
    if !network_available() {
        return;
    }
    let executable = fake_r();
    let session = Session::start(executable.display().to_string(), std::env::temp_dir());
    wait_for(&session, |e| matches!(e, Event::Ready { .. }));
    session.evaluate(10, "crash".into()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        for event in session.poll() {
            if let Event::Failed(message) = event {
                assert!(message.contains("exited") || message.contains("connection"));
                return;
            }
        }
        assert!(Instant::now() < deadline, "missing crash notification");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn invalid_runtime_is_reported() {
    let session = Session::start("/does/not/exist/R".into(), std::env::temp_dir());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        for event in session.poll() {
            if let Event::Failed(message) = event {
                assert!(message.contains("Install R"));
                return;
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn network_available() -> bool {
    match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(_) => true,
        Err(e) => {
            assert!(
                std::env::var_os("RSSTUDIO_REQUIRE_R").is_none(),
                "loopback unavailable: {e}"
            );
            eprintln!("Skipping session integration test: loopback unavailable: {e}");
            false
        }
    }
}

#[test]
fn real_r_persistent_session() {
    if !network_available() {
        return;
    }
    if std::process::Command::new("R")
        .arg("--version")
        .output()
        .is_err()
    {
        assert!(
            std::env::var_os("RSSTUDIO_REQUIRE_R").is_none(),
            "R is required for this test"
        );
        eprintln!("Skipping real R integration test: R is not installed");
        return;
    }
    let session = Session::start("R".into(), std::env::temp_dir());
    wait_for(&session, |e| matches!(e, Event::Ready { .. }));
    session.evaluate(1, "x <- 41".into()).unwrap();
    wait_for(&session, |e| matches!(e, Event::Done(1)));
    session.evaluate(2, "x + 1".into()).unwrap();
    let (_, mut output) = wait_for(&session, |e| matches!(e, Event::Done(2)));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !output.contains("42") && Instant::now() < deadline {
        for event in session.poll() {
            if let Event::Output(text) = event {
                output.push_str(&text);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        output.contains("42"),
        "expected visible R result, got {output:?}"
    );
    session.evaluate(3, "function(x) {".into()).unwrap();
    wait_for(&session, |e| matches!(e, Event::Incomplete(3, _)));
    session.evaluate(4, "stop('test error')".into()).unwrap();
    wait_for(&session, |e| matches!(e, Event::Error(4, _)));
    session.evaluate(5, "x + 1".into()).unwrap();
    wait_for(&session, |e| matches!(e, Event::Done(5)));
}
