use line_col::LineColLookup;
use std::borrow::Cow;
use urlencoding::encode;

const MAX_EXACT_MATCH_LENGTH: usize = 300;
const MIN_CONTEXT_WORDS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: usize,   // 1-indexed (Standard for line-col crate)
    pub column: usize, // 1-indexed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone)]
pub struct TextFragment {
    pub prefix: Option<String>,
    pub text_start: String,
    pub text_end: Option<String>,
    pub suffix: Option<String>,
}

impl TextFragment {
    pub fn to_hash_string(&self) -> String {
        let mut parts = Vec::new();
        if let Some(prefix) = &self.prefix {
            parts.push(format!("{}-", encode(prefix)));
        }
        parts.push(encode(&self.text_start).into_owned());
        if let Some(text_end) = &self.text_end {
            parts.push(encode(text_end).into_owned());
        }
        if let Some(suffix) = &self.suffix {
            parts.push(format!("-{}", encode(suffix)));
        }
        format!("#:~:text={}", parts.join(","))
    }
}

/// A parsed document representation supporting both Plain Text and HTML mapping
pub struct Document<'a> {
    pub source: &'a str,
    pub plain_text: String,
    lookup: LineColLookup<'a>,
    /// Maps HTML source byte index -> Plain Text byte index
    source_to_plain_map: Option<Vec<usize>>,
}

impl<'a> Document<'a> {
    pub fn from_plain_text(text: &'a str) -> Self {
        Self {
            source: text,
            plain_text: text.to_string(),
            lookup: LineColLookup::new(text),
            source_to_plain_map: None,
        }
    }

    pub fn from_html(html: &'a str) -> Self {
        let (plain_text, map) = Self::strip_html_and_map(html);
        Self {
            source: html,
            plain_text,
            lookup: LineColLookup::new(html),
            source_to_plain_map: Some(map),
        }
    }

    /// Strips HTML tags and creates a map of Source Byte Index -> Plain Text Byte Index.
    /// *Note: For production, consider using a full DOM parser to ignore `<script>` and `display: none` nodes.*
    fn strip_html_and_map(html: &str) -> (String, Vec<usize>) {
        let mut in_tag = false;
        let mut plain_text = String::with_capacity(html.len());
        let mut source_to_plain_map = vec![0; html.len() + 1];

        for (i, c) in html.char_indices() {
            if c == '<' {
                in_tag = true;
            }

            // Map the current HTML byte offset to the current plain_text byte offset
            for byte_offset in 0..c.len_utf8() {
                if i + byte_offset < source_to_plain_map.len() {
                    source_to_plain_map[i + byte_offset] = plain_text.len();
                }
            }

            if !in_tag {
                plain_text.push(c);
            }

            if c == '>' {
                in_tag = false;
            }
        }

        // Ensure the EOF boundary is mapped
        source_to_plain_map[html.len()] = plain_text.len();

        (plain_text, source_to_plain_map)
    }

    /// Converts a 1-indexed line/col into a byte offset using the `line_col` crate lookup.
    /// Then maps it to the plain text buffer if this is an HTML document.
    pub fn resolve_to_plain_text_offset(&self, pos: &Position) -> Option<usize> {
        // Find the source byte offset manually (line_col is usually byte -> line/col,
        // but we can reverse it by maintaining a line_starts index or iterating slightly).
        // For simplicity, we calculate the byte offset of the line start.
        let mut current_line = 1;
        let mut line_start_byte = 0;

        for (i, c) in self.source.char_indices() {
            if current_line == pos.line {
                line_start_byte = i;
                break;
            }
            if c == '\n' {
                current_line += 1;
            }
        }

        // Add the column offset (assuming pos.column is character-based)
        let source_byte = line_start_byte
            + self.source[line_start_byte..]
                .chars()
                .take(pos.column - 1)
                .map(|c| c.len_utf8())
                .sum::<usize>();

        if let Some(map) = &self.source_to_plain_map {
            map.get(source_byte).copied()
        } else {
            Some(source_byte)
        }
    }
}

pub struct FragmentGenerator<'a> {
    doc: Document<'a>,
}

impl<'a> FragmentGenerator<'a> {
    pub fn new(doc: Document<'a>) -> Self {
        Self { doc }
    }

    pub fn generate(&self, selection: Selection) -> Option<TextFragment> {
        let start_byte = self.doc.resolve_to_plain_text_offset(&selection.start)?;
        let end_byte = self.doc.resolve_to_plain_text_offset(&selection.end)?;

        if start_byte >= end_byte || end_byte > self.doc.plain_text.len() {
            return None;
        }

        let selected_text = &self.doc.plain_text[start_byte..end_byte];
        let cleaned_text = self.clean_text(selected_text);

        if cleaned_text.is_empty() {
            return None;
        }

        let (text_start, text_end) = self.determine_start_end(&cleaned_text);
        let is_unique = self.is_unique(&text_start, text_end.as_deref());

        let mut prefix = None;
        let mut suffix = None;

        if !is_unique {
            let (p, s) =
                self.calculate_context(start_byte, end_byte, &text_start, text_end.as_deref());
            prefix = p;
            suffix = s;
        }

        Some(TextFragment {
            prefix,
            text_start,
            text_end,
            suffix,
        })
    }

    fn clean_text(&self, text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn determine_start_end(&self, text: &str) -> (String, Option<String>) {
        if text.len() <= MAX_EXACT_MATCH_LENGTH {
            return (text.to_string(), None);
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        let start_words = words
            .iter()
            .take(MIN_CONTEXT_WORDS)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let end_words = words
            .iter()
            .rev()
            .take(MIN_CONTEXT_WORDS)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        (start_words, Some(end_words))
    }

    fn is_unique(&self, start: &str, end: Option<&str>) -> bool {
        let cleaned_doc = self.clean_text(&self.doc.plain_text);
        if let Some(end_text) = end {
            if let Some(idx) = cleaned_doc.find(start) {
                if cleaned_doc[idx + start.len()..].find(end_text).is_some() {
                    return cleaned_doc[idx + 1..].find(start).is_none();
                }
            }
            false
        } else {
            cleaned_doc.matches(start).count() == 1
        }
    }

    fn calculate_context(
        &self,
        start_byte: usize,
        end_byte: usize,
        text_start: &str,
        text_end: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let before_text = &self.doc.plain_text[..start_byte];
        let after_text = &self.doc.plain_text[end_byte..];

        let mut before_words: Vec<&str> = before_text.split_whitespace().rev().collect();
        let mut after_words: Vec<&str> = after_text.split_whitespace().collect();

        let mut prefix_words = Vec::new();
        let mut suffix_words = Vec::new();
        let mut is_unique = false;

        while !is_unique && (!before_words.is_empty() || !after_words.is_empty()) {
            if let Some(w) = before_words.first() {
                prefix_words.insert(0, *w);
                before_words.remove(0);
            }
            if let Some(w) = after_words.first() {
                suffix_words.push(*w);
                after_words.remove(0);
            }

            let p = prefix_words.join(" ");
            let s = suffix_words.join(" ");

            let pattern = format!("{} {}", p, text_start).trim().to_string();
            is_unique = self
                .clean_text(&self.doc.plain_text)
                .matches(&pattern)
                .count()
                == 1;
        }

        (
            if prefix_words.is_empty() {
                None
            } else {
                Some(prefix_words.join(" "))
            },
            if suffix_words.is_empty() {
                None
            } else {
                Some(suffix_words.join(" "))
            },
        )
    }
}
