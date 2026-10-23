//! A text being typed: its caret and selection, word and line moves, and
//! the input method's marked (still composing) text. No UI framework and no
//! keys here: the app turns the system's keys (`kadr_platform::keymap`)
//! into these moves, and the input method's calls, which count in UTF-16,
//! into `replace` and `mark`.

use std::ops::Range;

/// Where a caret move goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Move {
    Left,
    Right,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    Start,
    End,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Field {
    text: String,
    /// Byte offsets: where the caret is, and the selection's other end
    /// (the same when nothing is selected).
    caret: usize,
    anchor: usize,
    /// The input method's text being composed, if any.
    marked: Option<Range<usize>>,
}

impl Field {
    /// `text` with the caret at its end.
    pub fn new(text: &str) -> Field {
        Field { text: text.to_owned(), caret: text.len(), anchor: text.len(), marked: None }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selection, in order; empty at the caret when none.
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Moves the caret; `extend` keeps the selection's other end (Shift).
    pub fn go(&mut self, to: Move, extend: bool) {
        let sel = self.selection();
        // Left and Right with a selection and no Shift go to its edge.
        let at = match to {
            Move::Left if !extend && !sel.is_empty() => sel.start,
            Move::Right if !extend && !sel.is_empty() => sel.end,
            Move::Left => self.prev_char(self.caret),
            Move::Right => self.next_char(self.caret),
            Move::WordLeft => self.word_start(self.caret),
            Move::WordRight => self.word_end(self.caret),
            Move::LineStart => self.text[..self.caret].rfind('\n').map_or(0, |i| i + 1),
            Move::LineEnd => self.text[self.caret..].find('\n').map_or(self.text.len(), |i| self.caret + i),
            Move::Start => 0,
            Move::End => self.text.len(),
        };
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
        self.marked = None;
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
        self.marked = None;
    }

    /// Types `s` over the selection.
    pub fn insert(&mut self, s: &str) {
        let range = self.marked.take().unwrap_or_else(|| self.selection());
        self.splice(range, s);
    }

    /// Backspace: the selection, else the character before the caret.
    pub fn backspace(&mut self) {
        let sel = self.selection();
        let range = if sel.is_empty() { self.prev_char(self.caret)..self.caret } else { sel };
        self.splice(range, "");
    }

    /// Delete: the selection, else the character after the caret.
    pub fn delete(&mut self) {
        let sel = self.selection();
        let range = if sel.is_empty() { self.caret..self.next_char(self.caret) } else { sel };
        self.splice(range, "");
    }

    /// Option/Ctrl+Backspace: back to the start of the word.
    pub fn delete_word_back(&mut self) {
        let sel = self.selection();
        let range = if sel.is_empty() { self.word_start(self.caret)..self.caret } else { sel };
        self.splice(range, "");
    }

    // ------------------------------------------------ the input method's side

    /// The text in a UTF-16 range, as the input method asks for it.
    pub fn text_utf16(&self, range: Range<usize>) -> String {
        let r = self.bytes(range);
        self.text[r].to_owned()
    }

    pub fn selection_utf16(&self) -> Range<usize> {
        let r = self.selection();
        self.utf16(r.start)..self.utf16(r.end)
    }

    pub fn marked_utf16(&self) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.utf16(r.start)..self.utf16(r.end))
    }

    pub fn len_utf16(&self) -> usize {
        self.utf16(self.text.len())
    }

    /// The input method commits `s` over `range` (UTF-16), or over the
    /// marked text, or the selection.
    pub fn replace(&mut self, range: Option<Range<usize>>, s: &str) {
        let range = range.map(|r| self.bytes(r)).or_else(|| self.marked.take()).unwrap_or_else(|| self.selection());
        self.marked = None;
        self.splice(range, s);
    }

    /// The input method composes: `s` goes over `range` (or the marked
    /// text, or the selection) and stays marked, with the caret at
    /// `selected` inside it (UTF-16, from its start), else at its end.
    pub fn mark(&mut self, range: Option<Range<usize>>, s: &str, selected: Option<Range<usize>>) {
        let range = range.map(|r| self.bytes(r)).or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection());
        let start = range.start;
        self.splice(range, s);
        self.marked = (!s.is_empty()).then(|| start..start + s.len());
        if let Some(sel) = selected {
            let in_s = |u: usize| start + utf16_to_byte(s, u);
            self.anchor = in_s(sel.start);
            self.caret = in_s(sel.end);
        }
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Puts the caret and selection where the input method says (UTF-16).
    pub fn select_utf16(&mut self, range: Range<usize>) {
        let r = self.bytes(range);
        self.anchor = r.start;
        self.caret = r.end;
    }

    // ------------------------------------------------------------- inside

    fn splice(&mut self, range: Range<usize>, s: &str) {
        self.text.replace_range(range.clone(), s);
        self.caret = range.start + s.len();
        self.anchor = self.caret;
    }

    fn prev_char(&self, at: usize) -> usize {
        self.text[..at].char_indices().next_back().map_or(0, |(i, _)| i)
    }

    fn next_char(&self, at: usize) -> usize {
        self.text[at..].chars().next().map_or(at, |c| at + c.len_utf8())
    }

    /// Back over spaces and punctuation, then over the word.
    fn word_start(&self, at: usize) -> usize {
        let mut i = at;
        let before = |i: usize| self.text[..i].chars().next_back();
        while before(i).is_some_and(|c| !is_word(c)) {
            i = self.prev_char(i);
        }
        while before(i).is_some_and(is_word) {
            i = self.prev_char(i);
        }
        i
    }

    /// Over spaces and punctuation, then to the word's end.
    fn word_end(&self, at: usize) -> usize {
        let mut i = at;
        let after = |i: usize| self.text[i..].chars().next();
        while after(i).is_some_and(|c| !is_word(c)) {
            i = self.next_char(i);
        }
        while after(i).is_some_and(is_word) {
            i = self.next_char(i);
        }
        i
    }

    fn utf16(&self, byte: usize) -> usize {
        self.text[..byte].encode_utf16().count()
    }

    fn bytes(&self, r: Range<usize>) -> Range<usize> {
        let (a, b) = (utf16_to_byte(&self.text, r.start), utf16_to_byte(&self.text, r.end));
        a.min(b)..a.max(b)
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A UTF-16 offset in `s` as a byte offset, on a character boundary.
fn utf16_to_byte(s: &str, u: usize) -> usize {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n >= u {
            return i;
        }
        n += c.len_utf16();
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_goes_over_the_selection() {
        let mut f = Field::new("Hello world");
        f.go(Move::WordLeft, true);
        assert_eq!(&f.text()[f.selection()], "world");
        f.insert("мир");
        assert_eq!(f.text(), "Hello мир");
        f.select_all();
        f.insert("✓");
        assert_eq!((f.text(), f.caret()), ("✓", "✓".len()));
    }

    #[test]
    fn words_and_lines() {
        let mut f = Field::new("one, two_three\nfour");
        f.go(Move::WordLeft, false);
        assert_eq!(f.caret(), 15);
        f.go(Move::WordLeft, false);
        assert_eq!(f.caret(), 5, "over the newline to two_three");
        f.go(Move::WordRight, false);
        assert_eq!(f.caret(), 14);
        f.go(Move::LineStart, false);
        assert_eq!(f.caret(), 0);
        f.go(Move::LineEnd, false);
        assert_eq!(f.caret(), 14);
        f.go(Move::End, false);
        f.delete_word_back();
        assert_eq!(f.text(), "one, two_three\n");
        f.delete_word_back();
        assert_eq!(f.text(), "one, ");
        f.backspace();
        f.backspace();
        assert_eq!(f.text(), "one");
    }

    #[test]
    fn arrows_collapse_a_selection_to_its_side() {
        let mut f = Field::new("abc");
        f.select_all();
        f.go(Move::Left, false);
        assert_eq!((f.caret(), f.selection().is_empty()), (0, true));
        f.go(Move::Right, true);
        f.go(Move::Right, true);
        assert_eq!(f.selection(), 0..2);
        f.go(Move::Right, false);
        assert_eq!(f.caret(), 2);
    }

    #[test]
    fn the_input_method_composes_then_commits() {
        // UTF-16 offsets: "😀" is two units, "ж" one.
        let mut f = Field::new("😀ж");
        assert_eq!(f.len_utf16(), 3);
        f.mark(None, "ni", None);
        assert_eq!((f.text(), f.marked_utf16()), ("😀жni", Some(3..5)));
        f.mark(None, "你", Some(1..1));
        assert_eq!((f.text(), f.marked_utf16(), f.selection_utf16()), ("😀ж你", Some(3..4), 4..4));
        f.replace(None, "你好");
        assert_eq!((f.text(), f.marked()), ("😀ж你好", None));
        assert_eq!(f.text_utf16(0..2), "😀");
        f.select_utf16(2..3);
        f.replace(None, "");
        assert_eq!(f.text(), "😀你好");
    }
}
