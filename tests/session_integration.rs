#[allow(dead_code)]
#[path = "../src/packages.rs"]
mod packages;
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
 if kind == 'PACKAGES':
  send('STATUS',ident,'\n'.join(name + ('\t4.4.0\t1' if name == 'stats' else '\t-\t0') for name in value.split('\n'))); continue
 if kind in ('INSTALL','LOAD'):
  send('DONE',ident); continue
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
    let stopping = Instant::now();
    next.stop();
    wait_for(&next, |e| matches!(e, Event::Exited));
    assert!(stopping.elapsed() < Duration::from_secs(2));
    drop(next);
}

#[test]
fn package_requests_validate_names_and_route_responses() {
    if !network_available() {
        return;
    }
    let executable = fake_r();
    let session = Session::start(executable.display().to_string(), std::env::temp_dir());
    wait_for(&session, |e| matches!(e, Event::Ready { .. }));
    assert!(session.install_package(1, "dplyr;quit()", false).is_err());
    let names = vec!["stats".to_owned(), "dplyr".to_owned()];
    session.package_status(2, &names).unwrap();
    let (event, _) = wait_for(&session, |e| matches!(e, Event::PackageStatus(2, _)));
    assert!(
        matches!(event, Event::PackageStatus(_, payload) if payload.contains("stats\t4.4.0\t1"))
    );
    session.install_package(3, "DESeq2", true).unwrap();
    wait_for(&session, |e| matches!(e, Event::Done(3)));
    session.load_package(4, "stats").unwrap();
    wait_for(&session, |e| matches!(e, Event::Done(4)));
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
    let names = vec![
        "stats".to_owned(),
        "rsStudioDefinitelyMissingPackage".to_owned(),
    ];
    session.package_status(6, &names).unwrap();
    let (event, _) = wait_for(&session, |e| matches!(e, Event::PackageStatus(6, _)));
    if let Event::PackageStatus(_, payload) = event {
        assert!(
            payload
                .lines()
                .any(|row| row.starts_with("stats\t") && row.ends_with("\t1")),
            "{payload}"
        );
        assert!(
            payload.contains("rsStudioDefinitelyMissingPackage\t-\t0"),
            "{payload}"
        );
    }
    session.load_package(7, "stats").unwrap();
    wait_for(&session, |e| matches!(e, Event::Done(7)));
}

#[test]
fn real_r_environment_reports_loaded_data_and_refreshes() {
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
        return;
    }
    let session = Session::start("R".into(), std::env::temp_dir());
    wait_for(&session, |e| matches!(e, Event::Ready { .. }));
    session.evaluate(1, "assign('my data', data.frame(value = c(1, NA, 3), text = c('a', 'é', 'b')), .GlobalEnv)".into()).unwrap();
    wait_for(&session, |e| matches!(e, Event::Done(1)));
    session.environment(2).unwrap();
    let (event, _) = wait_for(&session, |e| matches!(e, Event::Environment(2, _)));
    let Event::Environment(_, payload) = event else {
        unreachable!()
    };
    let objects: serde_json::Value = serde_json::from_str(&payload).unwrap();
    let object = objects
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "my data")
        .unwrap();
    assert_eq!(object["length"], 3);
    assert_eq!(object["columns"][0]["missing_in_sample"], 1);
    assert_eq!(object["columns"][1]["preview"][1], "é");
    session
        .evaluate(3, "rm(list='my data', envir=.GlobalEnv)".into())
        .unwrap();
    wait_for(&session, |e| matches!(e, Event::Done(3)));
    session.environment(4).unwrap();
    let (event, _) = wait_for(&session, |e| matches!(e, Event::Environment(4, _)));
    let Event::Environment(_, payload) = event else {
        unreachable!()
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&payload).unwrap(),
        serde_json::json!([])
    );
}

#[test]
fn real_r_plots_are_captured_with_layers_and_history() {
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
        return;
    }
    let session = Session::start("R".into(), std::env::temp_dir());
    wait_for(&session, |e| matches!(e, Event::Ready { .. }));
    let run = |id, code: &str| {
        session.evaluate(id, code.into()).unwrap();
        let mut plots = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            for event in session.poll() {
                match event {
                    Event::Plot { page, png } => {
                        let image = image::load_from_memory(&png).expect("valid PNG");
                        assert_eq!((image.width(), image.height()), (960, 720));
                        let pixels = image.into_rgba8();
                        assert!(
                            pixels
                                .pixels()
                                .any(|p| p.0[0] < 220 && p.0[1] < 220 && p.0[2] < 220),
                            "plot image must contain visible marks"
                        );
                        plots.push((page, png));
                    }
                    Event::Done(done) if done == id => return plots,
                    Event::PlotError(e) | Event::Error(_, e) | Event::Failed(e) => {
                        panic!("plot failed: {e}")
                    }
                    _ => {}
                }
            }
            assert!(Instant::now() < deadline, "plot timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let first = run(1, "plot(1:10, main='first')");
    assert_eq!(first.len(), 1);
    let overlay = run(2, "points(1:10, col='red', pch=19)");
    assert_eq!(overlay.len(), 1);
    assert_eq!(first[0].0, overlay[0].0);
    assert_ne!(first[0].1, overlay[0].1);
    assert!(run(3, "x <- 2").is_empty());
    let later = run(4, "plot(10:1, main='second')");
    assert_eq!(later.len(), 1);
    assert_ne!(first[0].0, later[0].0);
    let looped = run(5, "for (i in 1:2) plot(1:3, main=as.character(i))");
    assert_eq!(looped.len(), 2);
    assert_ne!(looped[0].0, looped[1].0);
    let grid = run(6, "grid::grid.newpage(); grid::grid.text('grid plot')");
    assert!(!grid.is_empty());
    let panels = run(
        7,
        "par(mfrow=c(2,2)); for (i in 1:4) plot(1:4, main=as.character(i))",
    );
    let panel_page = panels.last().expect("multi-panel plot").0;
    let next_panel = run(8, "plot(1:5, main='next page')");
    assert_eq!(next_panel.len(), 1);
    assert_ne!(next_panel[0].0, panel_page);
    let ggplot = run(
        9,
        "par(mfrow=c(1,1)); if (requireNamespace('ggplot2', quietly=TRUE)) print(ggplot2::ggplot(mtcars, ggplot2::aes(mpg, wt)) + ggplot2::geom_point())",
    );
    if std::process::Command::new("Rscript")
        .args([
            "--vanilla",
            "-e",
            "quit(status=if(requireNamespace('ggplot2',quietly=TRUE)) 0 else 1)",
        ])
        .status()
        .is_ok_and(|s| s.success())
    {
        assert!(!ggplot.is_empty(), "ggplot2 output should be captured");
    }
    assert!(
        run(
            10,
            "png(file.path(tempdir(), 'explicit.png')); plot(1:4); dev.off()"
        )
        .is_empty(),
        "explicit output devices should not become pane plots"
    );
    assert!(run(11, "dev.off()").is_empty());
    assert!(
        run(
            12,
            "png(file.path(tempdir(), 'explicit-after-close.png')); plot(1:4); dev.off()"
        )
        .is_empty()
    );
    let fresh = run(13, "plot(1:3, main='new device')");
    assert!(!fresh.is_empty());
    let overlay = run(
        14,
        "par(new=TRUE); plot(1:3, col='blue', axes=FALSE, xlab='', ylab='')",
    );
    assert!(!overlay.is_empty());
    assert!(
        overlay
            .iter()
            .all(|(page, _)| *page == fresh.last().unwrap().0)
    );
}

#[test]
fn real_r_warning_options_without_network() {
    let result = std::process::Command::new("R")
        .args(["--vanilla", "--slave", "-f", "tests/r_warnings.R"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output();
    let output = match result {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("RSSTUDIO_REQUIRE_R").is_none(),
                "R is required"
            );
            eprintln!("Skipping warning checks: R is not installed");
            return;
        }
        Err(error) => panic!("Cannot execute R: {error}"),
    };
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn real_r_warning_arrives_during_evaluation() {
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
            "R is required"
        );
        eprintln!("Skipping warning timing check: R is not installed");
        return;
    }
    let session = Session::start("R".into(), std::env::temp_dir());
    wait_for(&session, |e| matches!(e, Event::Ready { .. }));
    session
        .evaluate(1, "sqrt(-1); Sys.sleep(1)".into())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_warning = false;
    loop {
        for event in session.poll() {
            match event {
                Event::Output(text) if text.contains("NaNs produced") => saw_warning = true,
                Event::Done(1) => {
                    assert!(saw_warning, "Warning was deferred until after completion");
                    return;
                }
                Event::Failed(message) | Event::Error(1, message) => panic!("R failed: {message}"),
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "Warning timing check timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn generated_r_worker_runs_without_network() {
    let path = std::env::temp_dir().join(format!(
        "rsstudio-worker-source-{}-{:?}.R",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::write(&path, session::worker_source()).unwrap();
    let result = std::process::Command::new("R")
        .args(["--vanilla", "--slave", "-f", "tests/r_worker.R", "--args"])
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output();
    fs::remove_file(path).unwrap();
    let output = match result {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("RSSTUDIO_REQUIRE_R").is_none(),
                "R is required"
            );
            eprintln!("Skipping generated worker checks: R is not installed");
            return;
        }
        Err(error) => panic!("Cannot execute R: {error}"),
    };
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
