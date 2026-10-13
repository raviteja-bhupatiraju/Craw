//! Byte-offset spans and byte-offset <-> LSP-position conversion.
//!
//! Everything internal to the compiler (lexer, parser, semantic model) uses
//! UTF-8 byte offsets into the source text as its only coordinate system.
//! `LineIndex` is the single place that translates those offsets to and from
//! LSP's `{line, UTF-16 character}` positions, so no other code needs to
//! reason about line counting or UTF-16 surrogate pairs.

/// A half-open byte range `[start, end)` into a source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A zero-based line/UTF-16-column position, matching the LSP `Position`
/// coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: u32,
    pub character: u32,
}

impl From<Pos> for tower_lsp::lsp_types::Position {
    fn from(p: Pos) -> Self {
        tower_lsp::lsp_types::Position {
            line: p.line,
            character: p.character,
        }
    }
}

impl From<tower_lsp::lsp_types::Position> for Pos {
    fn from(p: tower_lsp::lsp_types::Position) -> Self {
        Pos {
            line: p.line,
            character: p.character,
        }
    }
}

/// Maps UTF-8 byte offsets in a source string to and from LSP
/// `{line, UTF-16 character}` positions.
///
/// Built once per document version; `position`/`offset` are the only
/// entry points other modules should use for this conversion.
#[derive(Debug)]
pub struct LineIndex {
    text: String,
    /// Byte offset of the first byte of each line. Always starts with `0`;
    /// has one entry per line (a trailing `\n` starts a new, possibly empty,
    /// final line).
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (idx, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        Self {
            text: text.to_string(),
            line_starts,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// The line's text, excluding its trailing `\r?\n` (if any).
    pub fn line_text(&self, line: u32) -> &str {
        let idx = line as usize;
        let Some(&start) = self.line_starts.get(idx) else {
            return "";
        };
        let end = self
            .line_starts
            .get(idx + 1)
            .copied()
            .unwrap_or(self.text.len());
        let mut slice = &self.text[start..end];
        slice = slice.strip_suffix('\n').unwrap_or(slice);
        slice = slice.strip_suffix('\r').unwrap_or(slice);
        slice
    }

    /// Converts a byte offset (clamped to the text length) into a line/UTF-16
    /// column position.
    pub fn position(&self, offset: usize) -> Pos {
        let offset = offset.min(self.text.len());
        // Find the last line whose start is <= offset.
        let line = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx - 1,
        };
        let line_start = self.line_starts[line];
        let character = self.text[line_start..offset]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        Pos {
            line: line as u32,
            character,
        }
    }

    /// Converts a line/UTF-16 column position back into a byte offset.
    /// Returns `None` if `pos.line` is past the end of the document.
    /// `character` past the end of the line is clamped to the line's end
    /// (matching typical LSP client behavior for stale positions).
    pub fn offset(&self, pos: Pos) -> Option<usize> {
        let line = pos.line as usize;
        let &line_start = self.line_starts.get(line)?;
        let mut line_end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        // Clamp to before the line terminator, so an over-long column
        // never walks onto the next line.
        let line_text = &self.text[line_start..line_end];
        if line_text.ends_with("\r\n") {
            line_end -= 2;
        } else if line_text.ends_with('\n') {
            line_end -= 1;
        }
        let mut remaining_units = pos.character;
        let mut byte_idx = line_start;
        for c in self.text[line_start..line_end].chars() {
            if remaining_units == 0 {
                break;
            }
            let units = c.len_utf16() as u32;
            if units > remaining_units {
                break;
            }
            remaining_units -= units;
            byte_idx += c.len_utf8();
        }
        Some(byte_idx)
    }

    pub fn range(&self, span: Span) -> tower_lsp::lsp_types::Range {
        tower_lsp::lsp_types::Range {
            start: self.position(span.start).into(),
            end: self.position(span.end).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_clamps_past_end_column_before_the_newline() {
        let li = LineIndex::new("ab\ncd\r\nef");
        let at = |line, character| li.offset(Pos { line, character });
        assert_eq!(at(0, 99), Some(2));
        assert_eq!(at(1, 99), Some(5));
        assert_eq!(at(2, 99), Some(9));
    }

    #[test]
    fn ascii_positions() {
        let li = LineIndex::new("ab\ncd\n");
        assert_eq!(
            li.position(0),
            Pos {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            li.position(3),
            Pos {
                line: 1,
                character: 0
            }
        );
        assert_eq!(
            li.position(4),
            Pos {
                line: 1,
                character: 1
            }
        );
        assert_eq!(
            li.position(6),
            Pos {
                line: 2,
                character: 0
            }
        ); // EOF after trailing \n
        assert_eq!(
            li.offset(Pos {
                line: 1,
                character: 1
            }),
            Some(4)
        );
        assert_eq!(
            li.offset(Pos {
                line: 9,
                character: 0
            }),
            None
        );
    }

    #[test]
    fn utf16_columns() {
        // '≈' is 3 bytes / 1 UTF-16 unit; '🦀' is 4 bytes / 2 UTF-16 units
        let li = LineIndex::new("a≈b🦀c");
        assert_eq!(
            li.position(1),
            Pos {
                line: 0,
                character: 1
            }
        );
        assert_eq!(
            li.position(4),
            Pos {
                line: 0,
                character: 2
            }
        ); // 'b'
        assert_eq!(
            li.position(9),
            Pos {
                line: 0,
                character: 5
            }
        ); // 'c'
        assert_eq!(
            li.offset(Pos {
                line: 0,
                character: 5
            }),
            Some(9)
        );
    }

    #[test]
    fn crlf_lines() {
        let li = LineIndex::new("a\r\nb");
        assert_eq!(
            li.position(3),
            Pos {
                line: 1,
                character: 0
            }
        );
        assert_eq!(li.line_count(), 2);
        assert_eq!(li.line_text(0), "a");
    }

    #[test]
    fn range_conversion() {
        let li = LineIndex::new("x = 1\n");
        let r = li.range(Span { start: 4, end: 5 });
        assert_eq!(
            (r.start.line, r.start.character, r.end.character),
            (0, 4, 5)
        );
    }
}
