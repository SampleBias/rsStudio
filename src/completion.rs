//! Completion context, bounded cached results, and safe R identifier insertion.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Symbol,
    Member(String),
    Namespace(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub kind: Kind,
    pub prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub key: Key,
    pub buffer_id: u64,
    pub cursor: usize,
    pub start: usize, // character offsets for egui
    pub end: usize,
}

#[derive(Clone, Debug)]
pub struct Results {
    pub names: Vec<String>,
    pub more: bool,
}

#[derive(Default)]
pub struct Completion {
    pub query: Option<Query>,
    pub results: Vec<String>,
    pub selected: usize,
    pub visible: bool,
    changed: Option<Instant>,
    dismissed: Option<Key>,
    cache: VecDeque<(Key, Results)>,
    manual: Option<Query>,
    context_buffer: Option<u64>,
    context_cursor: (usize, usize),
    context_text: String,
    context_query: Option<Query>,
}

impl Completion {
    pub fn analyze(&mut self, text: &str, buffer_id: u64, cursor: (usize, usize)) -> Option<Query> {
        if self.context_buffer == Some(buffer_id)
            && self.context_cursor == cursor
            && self.context_text == text
        {
            return self.context_query.clone();
        }
        self.context_query = query(text, buffer_id, cursor);
        self.context_text.clear();
        self.context_text.push_str(text);
        self.context_buffer = Some(buffer_id);
        self.context_cursor = cursor;
        self.context_query.clone()
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn invalidate(&mut self) {
        self.cache.clear();
        self.results.clear();
        self.visible = false;
        self.changed = Some(Instant::now());
        self.dismissed = None;
        self.manual = None;
    }
    pub fn observe(&mut self, query: Option<Query>, now: Instant) {
        let key_changed = self.query.as_ref().map(|q| (&q.key, q.buffer_id))
            != query.as_ref().map(|q| (&q.key, q.buffer_id));
        if key_changed {
            self.changed = Some(now);
            self.dismissed = None;
            self.selected = 0;
        }
        self.query = query;
        if self
            .manual
            .as_ref()
            .is_some_and(|pending| Some(pending) != self.query.as_ref())
        {
            self.manual = None;
        }
        if key_changed {
            self.update_results();
        }
        if self.query.is_none() {
            self.visible = false;
        }
    }
    fn update_results(&mut self) {
        let Some(query) = &self.query else {
            self.results.clear();
            self.visible = false;
            return;
        };
        let exact = self
            .cache
            .iter()
            .rev()
            .find(|(key, _)| key == &query.key)
            .map(|(_, r)| r);
        let fallback = self
            .cache
            .iter()
            .rev()
            .find(|(key, r)| {
                key.kind == query.key.kind && !r.more && query.key.prefix.starts_with(&key.prefix)
            })
            .map(|(_, r)| r);
        self.results = exact
            .or(fallback)
            .map(|r| {
                r.names
                    .iter()
                    .filter(|n| n.starts_with(&query.key.prefix))
                    .take(100)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
        self.visible = !self.results.is_empty() && self.dismissed.as_ref() != Some(&query.key);
    }
    pub fn need_request(&self, now: Instant) -> Option<Key> {
        let q = self.query.as_ref()?;
        if q.key.prefix.is_empty() && matches!(q.key.kind, Kind::Symbol) {
            return None;
        }
        if self.cache.iter().any(|(key, _)| key == &q.key) {
            return None;
        }
        let wait = if !matches!(q.key.kind, Kind::Symbol) && q.key.prefix.is_empty() {
            Duration::ZERO
        } else {
            Duration::from_millis(200)
        };
        if now.duration_since(self.changed?) >= wait {
            Some(q.key.clone())
        } else {
            None
        }
    }
    pub fn receive(&mut self, key: Key, results: Results) {
        self.cache.retain(|(old, _)| old != &key);
        if self.cache.len() >= 32 {
            self.cache.pop_front();
        }
        self.cache.push_back((key, results));
        self.update_results();
    }
    pub fn dismiss(&mut self) {
        self.dismissed = self.query.as_ref().map(|q| q.key.clone());
        self.visible = false;
        self.manual = None;
    }
    pub fn open(&mut self) {
        if !self.results.is_empty() {
            self.dismissed = None;
            self.visible = true;
        }
    }
    pub fn move_selection(&mut self, delta: isize) {
        if !self.results.is_empty() {
            self.selected =
                (self.selected as isize + delta).rem_euclid(self.results.len() as isize) as usize;
        }
    }
    pub fn insertion(&self) -> Option<(Query, String)> {
        Some((
            self.query.clone()?,
            r_name(self.results.get(self.selected)?),
        ))
    }
    pub fn awaiting_lookup(&self) -> bool {
        self.query
            .as_ref()
            .is_some_and(|q| !self.cache.iter().any(|(key, _)| key == &q.key))
    }
    pub fn manual_pending(&self) -> bool {
        self.manual.is_some()
    }
    pub fn request_now(&mut self, now: Instant) {
        self.manual = self.query.clone();
        self.changed = now.checked_sub(Duration::from_millis(200));
    }
    pub fn manual_result(&mut self) -> Option<(Query, String)> {
        let q = self.manual.as_ref()?;
        if !self.cache.iter().any(|(key, _)| key == &q.key) {
            return None;
        }
        let q = self.manual.take()?;
        if self.results.is_empty() {
            Some((q, "\t".into()))
        } else if self.results.len() == 1 {
            self.insertion()
        } else {
            self.open();
            None
        }
    }
}

impl Key {
    pub fn payload(&self) -> String {
        let (mode, owner) = match &self.kind {
            Kind::Symbol => ("symbols", ""),
            Kind::Member(owner) => ("members", owner.as_str()),
            Kind::Namespace(owner) => ("namespace", owner.as_str()),
        };
        fn hex(s: &str) -> String {
            s.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
        }
        format!("{mode}\t{}\t{}", hex(owner), hex(&self.prefix))
    }
}

fn ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}
fn unescape_backticks(value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.peek(), Some('\\' | '`')) {
            result.push(chars.next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}
fn r_name(name: &str) -> String {
    let mut chars = name.chars();
    let first = chars.next();
    let valid_first = first.is_some_and(|c| c.is_alphabetic() || c == '.');
    let dot_digit = first == Some('.') && name.chars().nth(1).is_some_and(|c| c.is_ascii_digit());
    let reserved = matches!(
        name,
        "if" | "else"
            | "repeat"
            | "while"
            | "function"
            | "for"
            | "in"
            | "next"
            | "break"
            | "TRUE"
            | "FALSE"
            | "NULL"
            | "Inf"
            | "NaN"
            | "NA"
            | "NA_integer_"
            | "NA_real_"
            | "NA_complex_"
            | "NA_character_"
    );
    if valid_first && !dot_digit && !reserved && chars.all(ident) {
        name.to_owned()
    } else {
        format!("`{}`", name.replace('\\', "\\\\").replace('`', "\\`"))
    }
}

/// Find a simple symbol, dataset member, or loaded namespace at the caret.
/// Quotes/comments are excluded; arbitrary R expressions are never evaluated.
pub fn query(text: &str, buffer_id: u64, cursor: (usize, usize)) -> Option<Query> {
    if cursor.0 != cursor.1 {
        return None;
    }
    let caret = text
        .char_indices()
        .nth(cursor.0)
        .map_or(text.len(), |(i, _)| i);
    let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[line_start..caret];
    let mut quote = None;
    let mut quote_start = 0;
    let mut escaped = false;
    let mut comment = false;
    for (i, c) in text[..caret].char_indices() {
        if comment {
            if c == '\n' {
                comment = false;
            }
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if quote.is_some() && c == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '#' => comment = true,
            None if c == '\'' || c == '"' || c == '`' => {
                quote = Some(c);
                quote_start = i;
            }
            _ => {}
        }
    }
    if comment
        || matches!(quote, Some('\'' | '"'))
        || (quote == Some('`') && quote_start < line_start)
    {
        return None;
    }
    let (token_start, prefix, quoted) = if quote == Some('`') {
        (
            quote_start,
            unescape_backticks(&text[quote_start + 1..caret]),
            true,
        )
    } else {
        let start = line
            .char_indices()
            .rev()
            .find(|(_, c)| !ident(*c))
            .map_or(line_start, |(i, c)| line_start + i + c.len_utf8());
        (start, text[start..caret].to_owned(), false)
    };
    let before = text[line_start..token_start].trim_end();
    let (kind, operator) = if let Some(s) = before.strip_suffix('$') {
        (1, s.trim_end())
    } else if let Some(s) = before.strip_suffix("::") {
        if s.ends_with(':') {
            return None;
        }
        (2, s.trim_end())
    } else {
        (0, "")
    };
    let kind = if kind == 0 {
        if prefix.is_empty() {
            return None;
        }
        Kind::Symbol
    } else {
        let (owner_start, owner) = if operator.ends_with('`') {
            let end = operator.len() - 1;
            let start = operator[..end]
                .char_indices()
                .rev()
                .find(|(i, c)| {
                    *c == '`'
                        && operator[..*i]
                            .chars()
                            .rev()
                            .take_while(|c| *c == '\\')
                            .count()
                            % 2
                            == 0
                })?
                .0;
            (start, unescape_backticks(&operator[start + 1..end]))
        } else {
            let start = operator
                .char_indices()
                .rev()
                .find(|(_, c)| !ident(*c))
                .map_or(0, |(i, c)| i + c.len_utf8());
            (start, operator[start..].to_owned())
        };
        if operator[..owner_start]
            .trim_end()
            .chars()
            .last()
            .is_some_and(|c| matches!(c, '$' | '@' | ':' | ')' | ']'))
        {
            return None;
        }
        if owner.is_empty() {
            return None;
        }
        if kind == 1 {
            Kind::Member(owner)
        } else {
            Kind::Namespace(owner)
        }
    };
    let mut end = caret;
    if quoted {
        let mut escaped = false;
        for (offset, c) in text[caret..].char_indices() {
            if c == '\n' {
                break;
            }
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '`' {
                end = caret + offset + c.len_utf8();
                break;
            }
        }
    } else {
        for c in text[caret..].chars() {
            if !ident(c) {
                break;
            }
            end += c.len_utf8();
        }
    }
    Some(Query {
        key: Key { kind, prefix },
        buffer_id,
        cursor: cursor.0,
        start: text[..token_start].chars().count(),
        end: text[..end].chars().count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn q(s: &str) -> Query {
        query(s, 1, (s.chars().count(), s.chars().count())).unwrap()
    }
    #[test]
    fn finds_r_context_and_ignores_comments_and_strings() {
        assert_eq!(
            q("airquality$Te").key,
            Key {
                kind: Kind::Member("airquality".into()),
                prefix: "Te".into()
            }
        );
        assert_eq!(q("stats::me").key.kind, Kind::Namespace("stats".into()));
        assert_eq!(
            q("`my data`$`col na").key,
            Key {
                kind: Kind::Member("my data".into()),
                prefix: "col na".into()
            }
        );
        assert!(query("# airquality$", 1, (13, 13)).is_none());
        assert!(query("'airquality$", 1, (12, 12)).is_none());
        let multiline = "x <- 'a string\nairquality$Te";
        assert!(
            query(
                multiline,
                1,
                (multiline.chars().count(), multiline.chars().count())
            )
            .is_none()
        );
        assert_eq!(q("# previous comment\nmean").key.prefix, "mean");
        assert!(query("x$nested$", 1, (9, 9)).is_none());
    }
    #[test]
    fn unicode_ranges_and_non_syntactic_names_are_safe() {
        let s = "π <- d$Tempx";
        let first = query(s, 2, (11, 11)).unwrap();
        assert_eq!(first.key.prefix, "Temp");
        assert_eq!(first.end - first.start, 5);
        assert_eq!(r_name("column name"), "`column name`");
        assert_eq!(r_name("Temp"), "Temp");
        assert_eq!(r_name("if"), "`if`");
        assert_eq!(r_name("a`b"), "`a\\`b`");
        let s = "`my data`$`col name` + 1";
        let caret = "`my data`$`col".chars().count();
        let q = query(s, 2, (caret, caret)).unwrap();
        assert_eq!(q.key.prefix, "col");
        assert_eq!(
            &s[s.char_indices().nth(q.start).unwrap().0..s.char_indices().nth(q.end).unwrap().0],
            "`col name`"
        );
    }
    #[test]
    fn stale_responses_do_not_replace_current_suggestions() {
        let mut c = Completion::default();
        let now = Instant::now();
        let old = q("x$a");
        let new = q("x$b");
        c.observe(Some(old.clone()), now);
        c.observe(Some(new.clone()), now);
        c.receive(
            old.key,
            Results {
                names: vec!["alpha".into()],
                more: false,
            },
        );
        assert!(!c.visible);
        c.receive(
            new.key,
            Results {
                names: vec!["beta".into()],
                more: false,
            },
        );
        assert_eq!(c.results, ["beta"]);
    }
    #[test]
    fn manual_tab_waits_for_results_or_falls_back_to_indent() {
        let mut c = Completion::default();
        let now = Instant::now();
        let query = q("data$missing");
        c.observe(Some(query.clone()), now);
        c.request_now(now);
        assert_eq!(c.need_request(now), Some(query.key.clone()));
        assert!(c.manual_result().is_none());
        c.receive(
            query.key.clone(),
            Results {
                names: vec![],
                more: false,
            },
        );
        assert_eq!(c.manual_result(), Some((query, "\t".into())));
    }

    #[test]
    fn escape_cancels_a_pending_manual_completion() {
        let mut c = Completion::default();
        let query = q("data$Te");
        c.observe(Some(query.clone()), Instant::now());
        c.request_now(Instant::now());
        c.dismiss();
        c.receive(
            query.key,
            Results {
                names: vec!["Temp".into()],
                more: false,
            },
        );
        assert!(c.manual_result().is_none());
        assert!(!c.visible);
    }
}
