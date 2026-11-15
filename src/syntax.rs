//! Small, dependency-free R lexer for editor coloring; it is not an R parser.
use eframe::egui::{Color32, FontId, TextFormat, text::LayoutJob};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Plain,
    Comment,
    String,
    Number,
    Keyword,
    Operator,
}

fn spans(text: &str) -> Vec<(std::ops::Range<usize>, Kind)> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let start = i;
        let c = text[i..].chars().next().unwrap();
        i += c.len_utf8();
        let kind = if c == '#' {
            while i < text.len() && text.as_bytes()[i] != b'\n' {
                i += 1;
            }
            Kind::Comment
        } else if c == '"' || c == '\'' || c == '`' {
            let mut escaped = false;
            while i < text.len() {
                let next = text[i..].chars().next().unwrap();
                i += next.len_utf8();
                if escaped {
                    escaped = false;
                } else if next == '\\' {
                    escaped = true;
                } else if next == c {
                    break;
                }
            }
            Kind::String
        } else if c.is_ascii_digit() {
            while i < text.len()
                && (text.as_bytes()[i].is_ascii_alphanumeric()
                    || text.as_bytes()[i] == b'.'
                    || text.as_bytes()[i] == b'_')
            {
                i += 1;
            }
            Kind::Number
        } else if c.is_alphabetic() || c == '.' || c == '_' {
            while i < text.len() {
                let next = text[i..].chars().next().unwrap();
                if next.is_alphanumeric() || next == '.' || next == '_' {
                    i += next.len_utf8();
                } else {
                    break;
                }
            }
            if matches!(
                &text[start..i],
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
                    | "NA"
                    | "NaN"
                    | "Inf"
            ) {
                Kind::Keyword
            } else {
                Kind::Plain
            }
        } else if "+-*/^<>=!&|~:$@%".contains(c) {
            Kind::Operator
        } else {
            Kind::Plain
        };
        result.push((start..i, kind));
    }
    result
}

pub fn highlight(text: &str, dark: bool, size: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let font = FontId::monospace(size);
    for (range, kind) in spans(text) {
        let color = match (dark, kind) {
            (true, Kind::Comment) => Color32::from_rgb(151, 165, 145),
            (true, Kind::String) => Color32::from_rgb(167, 214, 146),
            (true, Kind::Number) => Color32::from_rgb(235, 192, 141),
            (true, Kind::Keyword) => Color32::from_rgb(193, 173, 255),
            (true, Kind::Operator) => Color32::from_rgb(135, 199, 231),
            (true, Kind::Plain) => Color32::from_rgb(225, 225, 230),
            (false, Kind::Comment) => Color32::from_rgb(73, 106, 66),
            (false, Kind::String) => Color32::from_rgb(35, 112, 58),
            (false, Kind::Number) => Color32::from_rgb(149, 75, 16),
            (false, Kind::Keyword) => Color32::from_rgb(101, 44, 156),
            (false, Kind::Operator) => Color32::from_rgb(14, 88, 133),
            (false, Kind::Plain) => Color32::from_rgb(35, 35, 40),
        };
        job.append(
            &text[range],
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifies_r_tokens_and_preserves_unicode() {
        let s = "if (π > 2) 'a#b' # comment\n";
        let parts = spans(s);
        let tokens: Vec<_> = parts.iter().map(|(r, k)| (&s[r.clone()], *k)).collect();
        assert!(tokens.contains(&("if", Kind::Keyword)));
        assert!(tokens.contains(&("π", Kind::Plain)));
        assert!(tokens.contains(&("2", Kind::Number)));
        assert!(tokens.contains(&("'a#b'", Kind::String)));
        assert!(tokens.contains(&("# comment", Kind::Comment)));
        assert_eq!(tokens.iter().map(|(s, _)| *s).collect::<String>(), s);
    }
}

/// Cache one active source document. Text comparison also catches programmatic
/// edits and TextEdit's intermediate layouts without requiring revision bookkeeping.
#[derive(Default)]
pub struct SourceCache {
    text: String,
    numbers: String,
    job: Option<(bool, u32, LayoutJob)>,
}
impl SourceCache {
    fn update_text(&mut self, text: &str) {
        if self.text != text || self.numbers.is_empty() {
            self.text.clear();
            self.text.push_str(text);
            let lines = text.lines().count() + usize::from(text.is_empty() || text.ends_with('\n'));
            self.numbers = (1..=lines).map(|n| format!("{n}\n")).collect();
            self.job = None;
        }
    }
    pub fn line_numbers(&mut self, text: &str) -> &str {
        self.update_text(text);
        &self.numbers
    }
    pub fn layout(
        &mut self,
        ui: &eframe::egui::Ui,
        text: &str,
        dark: bool,
        size: f32,
    ) -> std::sync::Arc<eframe::egui::Galley> {
        self.update_text(text);
        if self
            .job
            .as_ref()
            .is_none_or(|(cached_dark, cached_size, _)| {
                (*cached_dark, *cached_size) != (dark, size.to_bits())
            })
        {
            let mut job = highlight(text, dark, size);
            job.wrap.max_width = f32::INFINITY;
            self.job = Some((dark, size.to_bits(), job));
        }
        // egui owns the galley cache and invalidates it when the font atlas or
        // display scale changes. Retaining a Galley ourselves could paint stale UVs.
        ui.fonts_mut(|fonts| fonts.layout_job(self.job.as_ref().unwrap().2.clone()))
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use eframe::egui;
    #[test]
    fn layout_reuses_unchanged_source_and_invalidates_content_font_and_theme() {
        let ctx = egui::Context::default();
        let mut cache = SourceCache::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                use std::sync::Arc;
                assert_eq!(cache.line_numbers("x <- 1\n"), "1\n2\n");
                let first = cache.layout(ui, "x <- 1\n", true, 14.0);
                assert!(Arc::ptr_eq(
                    &first,
                    &cache.layout(ui, "x <- 1\n", true, 14.0)
                ));
                assert!(!Arc::ptr_eq(
                    &first,
                    &cache.layout(ui, "x <- 2\n", true, 14.0)
                ));
                let dark = cache.layout(ui, "x <- 2\n", true, 14.0);
                assert!(!Arc::ptr_eq(
                    &dark,
                    &cache.layout(ui, "x <- 2\n", false, 14.0)
                ));
                let small = cache.layout(ui, "x <- 2\n", false, 14.0);
                assert!(!Arc::ptr_eq(
                    &small,
                    &cache.layout(ui, "x <- 2\n", false, 18.0)
                ));
                assert_eq!(cache.line_numbers("x <- 2\ny <- 3"), "1\n2\n");
                assert_eq!(cache.line_numbers(""), "1\n");
            });
        });
    }
}
