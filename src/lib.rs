use std::borrow::Cow;
use urlencoding::encode;

const MAX_EXACT_MATCH_LENGTH: usize = 300;
const MIN_CONTEXT_WORDS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: usize,   // 0-indexed
    pub column: usize, // 0-indexed character/char offset
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
    /// Serializes the fragment into the standard URL hash format
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

pub struct FragmentGenerator<'a> {
    document: &'a str,
}

impl<'a> FragmentGenerator<'a> {
    pub fn new(document: &'a str) -> Self {
        Self { document }
    }

    /// Generates a Text Fragment from line-column pairs.
    pub fn generate(&self, selection: Selection) -> Option<TextFragment> {
        let (start_byte, end_byte) = self.resolve_offsets(&selection)?;
        if start_byte >= end_byte {
            return None;
        }

        let selected_text = &self.document[start_byte..end_byte];
        let cleaned_text = self.clean_text(selected_text);

        if cleaned_text.is_empty() {
            return None;
        }

        let (text_start, text_end) = self.determine_start_end(&cleaned_text);

        // Check if the current start/end combination is unique in the document
        let is_unique = self.is_unique(&text_start, text_end.as_deref());

        let mut prefix = None;
        let mut suffix = None;

        // If not unique, we must expand context (prefix/suffix) matching Chromium's behavior
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

    /// Converts line-column pairs into byte offsets within the string.
    fn resolve_offsets(&self, selection: &Selection) -> Option<(usize, usize)> {
        let mut current_line = 0;
        let mut line_start_byte = 0;

        let mut start_byte = None;
        let mut end_byte = None;

        for (byte_idx, c) in self.document.char_indices() {
            if current_line == selection.start.line && start_byte.is_none() {
                let char_offset = self.document[line_start_byte..]
                    .chars()
                    .take(selection.start.column)
                    .count();
                if char_offset == selection.start.column {
                    start_byte = Some(
                        line_start_byte
                            + self.document[line_start_byte..]
                                .chars()
                                .take(selection.start.column)
                                .map(|c| c.len_utf8())
                                .sum::<usize>(),
                    );
                }
            }

            if current_line == selection.end.line && end_byte.is_none() {
                let char_offset = self.document[line_start_byte..]
                    .chars()
                    .take(selection.end.column)
                    .count();
                if char_offset == selection.end.column {
                    end_byte = Some(
                        line_start_byte
                            + self.document[line_start_byte..]
                                .chars()
                                .take(selection.end.column)
                                .map(|c| c.len_utf8())
                                .sum::<usize>(),
                    );
                }
            }

            if c == '\n' {
                current_line += 1;
                line_start_byte = byte_idx + c.len_utf8();
            }
        }

        // Handle end of file cases
        if current_line == selection.end.line && end_byte.is_none() {
            end_byte = Some(self.document.len());
        }

        match (start_byte, end_byte) {
            (Some(s), Some(e)) => Some((s, e)),
            _ => None,
        }
    }

    /// Trims leading/trailing whitespace and normalizes internal whitespace.
    fn clean_text(&self, text: &str) -> String {
        let trimmed = text.trim();
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        words.join(" ")
    }

    /// Splits the text into start and end parts if it exceeds the maximum length.
    fn determine_start_end(&self, text: &str) -> (String, Option<String>) {
        if text.len() <= MAX_EXACT_MATCH_LENGTH {
            return (text.to_string(), None);
        }

        // Split by word boundaries approximating Chromium's limit logic
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

    /// Checks if the text (or start/end combo) appears exactly once in the document.
    fn is_unique(&self, start: &str, end: Option<&str>) -> bool {
        let cleaned_doc = self.clean_text(self.document);

        if let Some(end_text) = end {
            // Find start, then find end. Ensure no other start/end combinations exist.
            let first_start = cleaned_doc.find(start);
            if let Some(idx) = first_start {
                let remaining = &cleaned_doc[idx + start.len()..];
                if remaining.find(end_text).is_some() {
                    // Check if it appears AGAIN to see if it's non-unique
                    let after_first = &cleaned_doc[idx + 1..];
                    return after_first.find(start).is_none();
                }
            }
            false
        } else {
            cleaned_doc.matches(start).count() == 1
        }
    }

    /// Iteratively grabs preceding/succeeding words until the sequence is unique.
    fn calculate_context(
        &self,
        start_byte: usize,
        end_byte: usize,
        text_start: &str,
        text_end: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let before_text = &self.document[..start_byte];
        let after_text = &self.document[end_byte..];

        let mut before_words: Vec<&str> = before_text.split_whitespace().rev().collect();
        let mut after_words: Vec<&str> = after_text.split_whitespace().collect();

        let mut prefix_words = Vec::new();
        let mut suffix_words = Vec::new();

        let mut is_unique = false;

        // Chromium alternates adding to prefix and suffix until uniqueness is achieved
        while !is_unique && (!before_words.is_empty() || !after_words.is_empty()) {
            if let Some(w) = before_words.first() {
                prefix_words.insert(0, *w);
                before_words.remove(0);
            }

            if let Some(w) = after_words.first() {
                suffix_words.push(*w);
                after_words.remove(0);
            }

            let current_prefix = prefix_words.join(" ");
            let current_suffix = suffix_words.join(" ");

            // Re-check uniqueness with new context
            // (Simplified uniqueness check for brevity: in production, you evaluate the whole regex-like block)
            is_unique = self.check_context_uniqueness(
                &current_prefix,
                text_start,
                text_end,
                &current_suffix,
            );
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

    fn check_context_uniqueness(
        &self,
        prefix: &str,
        start: &str,
        _end: Option<&str>,
        suffix: &str,
    ) -> bool {
        // A naive but functional check to see if `prefix start ... end suffix` is unique.
        let pattern = format!("{} {}", prefix, start).trim().to_string();
        self.clean_text(self.document).matches(&pattern).count() == 1
    }
}
