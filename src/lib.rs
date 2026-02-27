use line_col::LineColLookup;
use std::borrow::Cow;
use urlencoding::{decode, encode};

const MAX_EXACT_MATCH_LENGTH: usize = 300;
const MIN_CONTEXT_WORDS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: usize,   // 1-indexed
    pub column: usize, // 1-indexed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    /// Forces the generator to include at least this many prefix words, even if already unique.
    pub min_prefix_words: usize,
    /// Forces the generator to include at least this many suffix words, even if already unique.
    pub min_suffix_words: usize,
}

impl GenerateOptions {
    pub fn new(min_prefix_words: usize, min_suffix_words: usize) -> Self {
        Self {
            min_prefix_words,
            min_suffix_words,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

pub struct Document<'a> {
    pub source: &'a str,
    pub plain_text: String,
    lookup: LineColLookup<'a>,
    source_to_plain_map: Option<Vec<usize>>,
    plain_to_source_map: Option<Vec<usize>>,
}

impl<'a> Document<'a> {
    pub fn from_plain_text(text: &'a str) -> Self {
        Self {
            source: text,
            plain_text: text.to_string(),
            lookup: LineColLookup::new(text),
            source_to_plain_map: None,
            plain_to_source_map: None,
        }
    }

    pub fn from_html(html: &'a str) -> Self {
        let (plain_text, s2p, p2s) = Self::strip_html_and_map(html);
        Self {
            source: html,
            plain_text,
            lookup: LineColLookup::new(html),
            source_to_plain_map: Some(s2p),
            plain_to_source_map: Some(p2s),
        }
    }

    fn strip_html_and_map(html: &str) -> (String, Vec<usize>, Vec<usize>) {
        let mut in_tag = false;
        let mut plain_text = String::with_capacity(html.len());

        let mut source_to_plain_map = vec![0; html.len() + 1];
        let mut plain_to_source_map = Vec::with_capacity(html.len());

        for (i, c) in html.char_indices() {
            if c == '<' {
                in_tag = true;
            }

            for byte_offset in 0..c.len_utf8() {
                if i + byte_offset < source_to_plain_map.len() {
                    source_to_plain_map[i + byte_offset] = plain_text.len();
                }
            }

            if !in_tag {
                for byte_offset in 0..c.len_utf8() {
                    plain_to_source_map.push(i + byte_offset);
                }
                plain_text.push(c);
            }

            if c == '>' {
                in_tag = false;
            }
        }

        source_to_plain_map[html.len()] = plain_text.len();
        (plain_text, source_to_plain_map, plain_to_source_map)
    }

    pub fn resolve_to_plain_text_offset(&self, pos: &Position) -> Option<usize> {
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

    /// Converts a plain text byte offset back into a line-column pair in the original source
    pub fn resolve_to_source_position(&self, plain_text_byte: usize) -> Option<Position> {
        let source_byte = if let Some(map) = &self.plain_to_source_map {
            *map.get(plain_text_byte)?
        } else {
            plain_text_byte
        };

        let (line, col) = self.lookup.get(source_byte);
        Some(Position { line, column: col })
    }
}

pub struct FragmentEngine<'a> {
    doc: Document<'a>,
}

impl<'a> FragmentEngine<'a> {
    pub fn new(doc: Document<'a>) -> Self {
        Self { doc }
    }

    /// Generates a fragment, with an optional robustness configuration to enforce prefix/suffix inclusion.
    pub fn generate(
        &self,
        selection: Selection,
        options: Option<GenerateOptions>,
    ) -> Option<TextFragment> {
        let opts = options.unwrap_or_default();

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

        // Context calculation handles both uniqueness AND the robustness minimums
        let (prefix, suffix) = self.calculate_context(
            start_byte,
            end_byte,
            &text_start,
            text_end.as_deref(),
            &opts,
        );

        Some(TextFragment {
            prefix,
            text_start,
            text_end,
            suffix,
        })
    }

    /// Resolves a Text Fragment back into a Line-Column Selection in the original source.
    pub fn resolve_fragment(&self, fragment: &TextFragment) -> Option<Selection> {
        // Simplified matching: In a strict Chromium port, this would handle complex whitespace folding.
        // Here, we find matches of `text_start` and expand to `text_end` if present.

        let mut match_start_byte = None;
        let mut match_end_byte = None;

        let search_text = &self.doc.plain_text;

        // Find all occurrences of text_start
        let start_indices = search_text
            .match_indices(&fragment.text_start)
            .map(|(i, _)| i);

        for start_idx in start_indices {
            dbg!(start_idx);
            let mut is_valid = true;

            // 1. Verify Prefix
            if let Some(prefix) = &fragment.prefix {
                let before_text = &search_text[..start_idx];
                dbg!(before_text);
                if !before_text.trim_end().ends_with(prefix) {
                    is_valid = false;
                }
            }

            if !is_valid {
                continue;
            }

            // 2. Find text_end (or just use text_start's end)
            let mut end_idx = start_idx + fragment.text_start.len();

            if let Some(text_end) = &fragment.text_end {
                let after_start_text = &search_text[end_idx..];
                dbg!(after_start_text);
                if let Some(end_offset) = after_start_text.find(text_end) {
                    end_idx += end_offset + text_end.len();
                } else {
                    is_valid = false;
                }
            }

            if !is_valid {
                continue;
            }

            // 3. Verify Suffix
            if let Some(suffix) = &fragment.suffix {
                let after_text = &search_text[end_idx..];
                dbg!(after_text);
                if !after_text.trim_start().starts_with(suffix) {
                    is_valid = false;
                }
            }

            if is_valid {
                match_start_byte = Some(start_idx);
                match_end_byte = Some(end_idx);
                break; // Found the unique match
            }
        }

        let start_pos = self.doc.resolve_to_source_position(match_start_byte?)?;
        let end_pos = self.doc.resolve_to_source_position(match_end_byte?)?;
        // .resolve_to_source_position(match_end_byte.map(|b| b - 1)?)?;

        Some(Selection {
            start: start_pos,
            end: end_pos,
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

    fn calculate_context(
        &self,
        start_byte: usize,
        end_byte: usize,
        text_start: &str,
        text_end: Option<&str>,
        opts: &GenerateOptions,
    ) -> (Option<String>, Option<String>) {
        let before_text = &self.doc.plain_text[..start_byte];
        let after_text = &self.doc.plain_text[end_byte..];

        let mut before_words: Vec<&str> = before_text.split_whitespace().rev().collect();
        let mut after_words: Vec<&str> = after_text.split_whitespace().collect();

        let mut prefix_words = Vec::new();
        let mut suffix_words = Vec::new();
        let mut is_unique = false;

        // Loop until unique AND robustness minimums are met
        while (!is_unique
            || prefix_words.len() < opts.min_prefix_words
            || suffix_words.len() < opts.min_suffix_words)
            && (!before_words.is_empty() || !after_words.is_empty())
        {
            if let Some(w) = before_words.first() {
                if prefix_words.len() < opts.min_prefix_words
                    || (!is_unique && prefix_words.len() <= suffix_words.len())
                {
                    prefix_words.insert(0, *w);
                    before_words.remove(0);
                }
            }
            if let Some(w) = after_words.first() {
                if suffix_words.len() < opts.min_suffix_words
                    || (!is_unique && suffix_words.len() < prefix_words.len())
                {
                    suffix_words.push(*w);
                    after_words.remove(0);
                }
            }

            let p = prefix_words.join(" ");
            let s = suffix_words.join(" ");

            // Check uniqueness if we aren't already unique
            if !is_unique {
                let pattern = format!("{} {}", p, text_start).trim().to_string();
                is_unique = self
                    .clean_text(&self.doc.plain_text)
                    .matches(&pattern)
                    .count()
                    == 1;
            }
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
