//! On-demand CRAN package metadata, independent of the R execution queue.
use crate::packages::valid_name;
use reqwest::{StatusCode, blocking::Client};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    time::Duration,
};

const PAGE_PREFIX_LIMIT: u64 = 64 * 1024;

#[derive(Debug)]
pub struct Metadata {
    pub title: String,
    pub description: String,
}

#[derive(Debug)]
pub enum State {
    Loading,
    Ready(Metadata),
    Missing,
    Failed(String),
}

#[derive(Default)]
pub struct Lookup {
    states: BTreeMap<String, State>,
    worker: Option<Worker>,
}

impl Lookup {
    pub fn get(&self, name: &str) -> Option<&State> {
        self.states.get(name)
    }

    pub fn forget(&mut self, name: &str) {
        self.states.remove(name);
    }

    pub fn request(&mut self, name: &str) {
        if !valid_name(name)
            || matches!(self.get(name), Some(State::Loading | State::Ready(_)))
            || (self.states.len() >= 200 && !self.states.contains_key(name))
        {
            return;
        }
        let worker = self.worker.get_or_insert_with(Worker::http);
        let state = match worker.requests.try_send(name.to_owned()) {
            Ok(()) => State::Loading,
            Err(_) => State::Failed("The CRAN lookup queue is busy. Retry shortly.".into()),
        };
        self.states.insert(name.to_owned(), state);
    }

    pub fn poll(&mut self) {
        for _ in 0..8 {
            let Some(worker) = &self.worker else { break };
            match worker.results.try_recv() {
                Ok((name, state)) => {
                    if let Some(current) = self.states.get_mut(&name) {
                        *current = state;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    for state in self.states.values_mut() {
                        if matches!(state, State::Loading) {
                            *state =
                                State::Failed("CRAN lookup stopped. Retry to reconnect.".into());
                        }
                    }
                    break;
                }
            }
        }
    }

    #[cfg(test)]
    pub fn with_fetch(fetch: impl FnMut(&str) -> State + Send + 'static) -> Self {
        Self {
            worker: Some(Worker::spawn(fetch)),
            ..Self::default()
        }
    }
}

struct Worker {
    requests: SyncSender<String>,
    results: Receiver<(String, State)>,
}

impl Worker {
    fn spawn(mut fetch: impl FnMut(&str) -> State + Send + 'static) -> Self {
        let (requests, input) = mpsc::sync_channel::<String>(4);
        let (output, results) = mpsc::sync_channel(4);
        std::thread::spawn(move || {
            while let Ok(name) = input.recv() {
                let result = fetch(&name);
                if output.send((name, result)).is_err() {
                    break;
                }
            }
        });
        Self { requests, results }
    }

    fn http() -> Self {
        let mut client = None;
        Self::spawn(move |name| {
            if client.is_none() {
                match Client::builder()
                    .https_only(true)
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(3))
                    .timeout(Duration::from_secs(10))
                    .user_agent(concat!("rsStudio/", env!("CARGO_PKG_VERSION")))
                    .build()
                {
                    Ok(value) => client = Some(value),
                    Err(_) => {
                        return State::Failed("Could not initialize the CRAN connection.".into());
                    }
                }
            }
            match client
                .as_ref()
                .unwrap()
                .get(format!(
                    "https://cran.r-project.org/web/packages/{name}/index.html"
                ))
                .send()
            {
                Ok(response) => read_response(name, response.status(), response),
                Err(_) => {
                    State::Failed("Could not reach CRAN. Check your connection and retry.".into())
                }
            }
        })
    }
}

pub fn package_url(name: &str) -> String {
    format!("https://cran.r-project.org/package={name}")
}

fn read_response(name: &str, status: StatusCode, reader: impl Read) -> State {
    if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
        return State::Missing;
    }
    if !status.is_success() {
        return State::Failed(format!(
            "CRAN returned HTTP {}. Retry later.",
            status.as_u16()
        ));
    }
    // CRAN puts the title and description first. Avoid downloading the potentially
    // large reverse-dependency lists at the end of a package page.
    let mut bytes = Vec::new();
    if reader
        .take(PAGE_PREFIX_LIMIT)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return State::Failed("Could not read the CRAN response. Retry later.".into());
    }
    let html = match std::str::from_utf8(&bytes) {
        Ok(html) => Some(html),
        // A large page may be cut inside a Unicode character in its dependency
        // list, after the complete metadata. Keep the valid prefix in that case.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    };
    match html.and_then(|html| parse_page(name, html)) {
        Some(metadata) => State::Ready(metadata),
        None => State::Failed("CRAN did not return readable information for this package.".into()),
    }
}

fn element<'a>(html: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let lower = html.to_ascii_lowercase();
    let opening = format!("<{tag}");
    let closing = format!("</{tag}>");
    let mut offset = 0;
    while let Some(start) = lower[offset..].find(&opening) {
        let start = offset + start + opening.len();
        if lower
            .as_bytes()
            .get(start)
            .is_some_and(|c| *c == b'>' || c.is_ascii_whitespace())
        {
            let body = start + lower[start..].find('>')? + 1;
            let end = body + lower[body..].find(&closing)?;
            return Some((&html[body..end], &html[end + closing.len()..]));
        }
        offset = start;
    }
    None
}

fn parse_page(name: &str, html: &str) -> Option<Metadata> {
    let (heading, rest) = element(html, "h2")?;
    let heading = plain_text(heading);
    let (package, title) = heading.split_once(':')?;
    let title = title.trim();
    if package.trim() != name || title.is_empty() || title.len() > 1024 {
        return None;
    }
    let (paragraph, _) = element(rest, "p")?;
    let description = plain_text(paragraph);
    if description.is_empty() || description.len() > 16 * 1024 {
        return None;
    }
    Some(Metadata {
        title: title.to_owned(),
        description,
    })
}

fn plain_text(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag
                    .split_ascii_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_matches('/')
                    .to_ascii_lowercase();
                if matches!(name.as_str(), "br" | "p" | "div" | "li") {
                    text.push(' ');
                }
            }
            _ if !in_tag => text.push(ch),
            _ => tag.push(ch),
        }
    }
    let mut decoded = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(end) = rest.find(';').filter(|n| *n <= 16)
            && let Some(ch) = entity(&rest[1..end])
        {
            decoded.push(ch);
            rest = &rest[end + 1..];
        } else {
            decoded.push('&');
            rest = &rest[1..];
        }
    }
    decoded.push_str(rest);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        _ => {
            let code =
                if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    name.strip_prefix('#')?.parse().ok()?
                };
            let ch = char::from_u32(code)?;
            if ch.is_control() {
                return None;
            }
            ch
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Instant,
    };

    // Representative CRAN page structure, with a short synthetic description.
    const PAGE: &str = "<html><body><h2>torch: Tensors and Neural Networks with 'GPU' Acceleration</h2>\n<p>Train neural networks in R &amp; work with <a href='example'>tensors</a>.</p></body></html>";

    fn ready() -> State {
        State::Ready(parse_page("torch", PAGE).unwrap())
    }

    fn wait_for(lookup: &mut Lookup, name: &str, predicate: impl Fn(&State) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            lookup.poll();
            if lookup.get(name).is_some_and(&predicate) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Lookup did not complete: {:?}",
                lookup.get(name)
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn extracts_the_requested_package_and_decodes_description_markup() {
        let metadata = parse_page("torch", PAGE).unwrap();
        assert!(metadata.title.starts_with("Tensors and Neural Networks"));
        assert_eq!(
            metadata.description,
            "Train neural networks in R & work with tensors."
        );
        let page = "<H2 id='name'>ranger: Fast &#x2018;Forests&#8217; &amp; Trees</H2><PRE>skip</PRE><P class='desc'>\n Fast&nbsp;models <em>in R</em>.\n</P>";
        let metadata = parse_page("ranger", page).unwrap();
        assert_eq!(metadata.title, "Fast ‘Forests’ & Trees");
        assert_eq!(metadata.description, "Fast models in R.");
        assert!(parse_page("other", PAGE).is_none());
        assert!(parse_page("torch", "<h2>torch: Wrong</h2><p>truncated").is_none());
        assert!(parse_page("torch", "<html>package archived</html>").is_none());
        assert_eq!(
            package_url("torch"),
            "https://cran.r-project.org/package=torch"
        );
    }

    #[test]
    fn distinguishes_missing_packages_from_transient_failures() {
        let page = PAGE.as_bytes();
        assert!(matches!(
            read_response("torch", StatusCode::OK, page),
            State::Ready(_)
        ));
        assert!(matches!(
            read_response("torch", StatusCode::NOT_FOUND, page),
            State::Missing
        ));
        assert!(matches!(
            read_response("torch", StatusCode::GONE, page),
            State::Missing
        ));
        assert!(matches!(
            read_response("torch", StatusCode::SERVICE_UNAVAILABLE, page),
            State::Failed(_)
        ));
        assert!(matches!(
            read_response("torch", StatusCode::OK, &b"<html>Error page</html>"[..]),
            State::Failed(_)
        ));
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::TimedOut.into())
            }
        }
        assert!(matches!(
            read_response("torch", StatusCode::OK, Broken),
            State::Failed(_)
        ));
    }

    #[test]
    fn reads_only_a_bounded_prefix_even_with_a_partial_unicode_tail() {
        let mut page = PAGE.as_bytes().to_vec();
        page.resize(PAGE_PREFIX_LIMIT as usize - 1, b' ');
        page.extend_from_slice("π".as_bytes());
        let mut reader = io::Cursor::new(page);
        assert!(matches!(
            read_response("torch", StatusCode::OK, &mut reader),
            State::Ready(_)
        ));
        assert_eq!(reader.position(), PAGE_PREFIX_LIMIT);
    }

    #[test]
    fn lookup_is_nonblocking_and_caches_duplicate_requests() {
        let (started, received) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let count = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&count);
        let mut lookup = Lookup::with_fetch(move |name| {
            calls.fetch_add(1, Ordering::SeqCst);
            started.send(name.to_owned()).unwrap();
            gate.recv_timeout(Duration::from_secs(2)).unwrap();
            ready()
        });
        lookup.request("torch");
        lookup.request("torch");
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            "torch"
        );
        lookup.poll();
        assert!(matches!(lookup.get("torch"), Some(State::Loading)));
        release.send(()).unwrap();
        wait_for(&mut lookup, "torch", |s| matches!(s, State::Ready(_)));
        lookup.request("torch");
        assert_eq!(count.load(Ordering::SeqCst), 1);
        lookup.request("../not-a-package");
        assert!(lookup.get("../not-a-package").is_none());
    }

    #[test]
    fn failed_lookups_can_be_retried() {
        let mut attempts = 0;
        let mut lookup = Lookup::with_fetch(move |_| {
            attempts += 1;
            if attempts == 1 {
                State::Failed("offline".into())
            } else {
                ready()
            }
        });
        lookup.request("torch");
        wait_for(&mut lookup, "torch", |s| matches!(s, State::Failed(_)));
        lookup.request("torch");
        wait_for(&mut lookup, "torch", |s| matches!(s, State::Ready(_)));
    }

    #[test]
    fn a_full_queue_reports_a_retriable_error() {
        let (started, received) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut first = true;
        let mut lookup = Lookup::with_fetch(move |_| {
            if first {
                first = false;
                started.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            ready()
        });
        lookup.request("torch");
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        for name in ["one", "two", "three", "four", "five"] {
            lookup.request(name);
        }
        assert!(matches!(lookup.get("five"), Some(State::Failed(_))));
        release.send(()).unwrap();
    }

    #[test]
    fn late_lookup_results_do_not_restore_a_removed_entry() {
        let (requests, _input) = mpsc::sync_channel(1);
        let (output, results) = mpsc::sync_channel(1);
        let mut lookup = Lookup {
            states: BTreeMap::from([("torch".into(), State::Loading)]),
            worker: Some(Worker { requests, results }),
        };
        lookup.forget("torch");
        output.send(("torch".into(), ready())).unwrap();
        lookup.poll();
        assert!(lookup.get("torch").is_none());
    }
}
