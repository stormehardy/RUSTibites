//! The caret and the selection of a `TMP_InputField`: a position between characters and, while selecting, the other end of the selection (`caretPosition`,
//! `selectionAnchorPosition`). Positions count characters, not bytes. Left / Right / Home / End move the caret (with Shift they extend the selection; without
//! it a selection collapses to its near end), Backspace / Delete remove the selection or the character beside the caret, typing replaces the selection,
//! Ctrl + A selects everything, a click puts the caret under the pointer and a drag selects.

/// The caret of a field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Caret {
    /// The caret's place (characters from the start).
    pub pos: usize,
    /// The other end of the selection, when something is selected.
    pub anchor: Option<usize>,
}

fn byte_of(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(index, _)| index)
}

impl Caret {
    /// A caret at the end of `text`.
    #[must_use]
    pub fn at_end(text: &str) -> Self {
        Self { pos: text.chars().count(), anchor: None }
    }

    /// The selected characters as `(from, to)`, when any.
    #[must_use]
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.anchor.filter(|anchor| *anchor != self.pos).map(|anchor| (anchor.min(self.pos), anchor.max(self.pos)))
    }

    /// Selects the whole text (`onFocusSelectAll`, Ctrl + A).
    pub fn select_all(&mut self, text: &str) {
        self.anchor = Some(0);
        self.pos = text.chars().count();
    }

    /// Keeps the caret inside `text` (after the text was changed from outside).
    pub fn clamp(&mut self, text: &str) {
        let len = text.chars().count();
        self.pos = self.pos.min(len);
        self.anchor = self.anchor.map(|anchor| anchor.min(len));
    }

    fn remove_selection(&mut self, text: &mut String) -> bool {
        let Some((from, to)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        let (a, b) = (byte_of(text, from), byte_of(text, to));
        text.replace_range(a..b, "");
        self.pos = from;
        self.anchor = None;
        true
    }

    /// Typed text replaces the selection, else goes in at the caret; control characters are dropped.
    pub fn insert(&mut self, text: &mut String, typed: &str) {
        for character in typed.chars().filter(|character| !character.is_control()) {
            self.remove_selection(text);
            let at = byte_of(text, self.pos);
            text.insert(at, character);
            self.pos += 1;
        }
    }

    /// A line break at the caret (a multi-line field; `insert` drops control characters).
    pub fn insert_newline(&mut self, text: &mut String) {
        self.remove_selection(text);
        let at = byte_of(text, self.pos);
        text.insert(at, '\n');
        self.pos += 1;
    }

    /// Backspace: the selection, or the character before the caret.
    pub fn backspace(&mut self, text: &mut String) {
        if self.remove_selection(text) || self.pos == 0 {
            return;
        }
        let (a, b) = (byte_of(text, self.pos - 1), byte_of(text, self.pos));
        text.replace_range(a..b, "");
        self.pos -= 1;
    }

    /// Delete: the selection, or the character after the caret.
    pub fn delete(&mut self, text: &mut String) {
        if self.remove_selection(text) || self.pos >= text.chars().count() {
            return;
        }
        let (a, b) = (byte_of(text, self.pos), byte_of(text, self.pos + 1));
        text.replace_range(a..b, "");
    }

    fn move_to(&mut self, to: usize, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.pos);
            }
        } else {
            self.anchor = None;
        }
        self.pos = to;
    }

    /// Left arrow.
    pub fn left(&mut self, extend: bool) {
        match (self.selection(), extend) {
            (Some((from, _)), false) => {
                self.anchor = None;
                self.pos = from;
            }
            _ => self.move_to(self.pos.saturating_sub(1), extend),
        }
    }

    /// Right arrow.
    pub fn right(&mut self, text: &str, extend: bool) {
        match (self.selection(), extend) {
            (Some((_, to)), false) => {
                self.anchor = None;
                self.pos = to;
            }
            _ => self.move_to((self.pos + 1).min(text.chars().count()), extend),
        }
    }

    /// Home.
    pub fn home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    /// End.
    pub fn end(&mut self, text: &str, extend: bool) {
        self.move_to(text.chars().count(), extend);
    }

    /// A press puts the caret at `pos` (Shift keeps the old end as the anchor); a drag then extends from it.
    pub fn press(&mut self, pos: usize, extend: bool) {
        self.move_to(pos, extend);
    }

    /// The pointer moved with the button held: the selection reaches `pos`.
    pub fn drag(&mut self, pos: usize) {
        if self.anchor.is_none() {
            self.anchor = Some(self.pos);
        }
        self.pos = pos;
    }
}

/// The keys that move or edit inside a text field this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct EditKeys {
    /// Backspace.
    pub backspace: bool,
    /// Delete.
    pub delete: bool,
    /// Left arrow.
    pub left: bool,
    /// Right arrow.
    pub right: bool,
    /// Home.
    pub home: bool,
    /// End.
    pub end: bool,
    /// Ctrl + A.
    pub select_all: bool,
    /// Shift is held (arrows extend the selection).
    pub shift: bool,
}

/// One frame of editing `text` with its `caret`: typed characters (control characters dropped, at most `max` in the text) go in at the caret, then the keys.
pub fn edit(text: &mut String, caret: &mut Caret, typed: &str, max: usize, keys: EditKeys) {
    caret.clamp(text);
    for character in typed.chars().filter(|character| !character.is_control()) {
        if caret.selection().is_some() || text.chars().count() < max {
            caret.insert(text, &character.to_string());
        }
    }
    if keys.backspace {
        caret.backspace(text);
    }
    if keys.delete {
        caret.delete(text);
    }
    if keys.left {
        caret.left(keys.shift);
    }
    if keys.right {
        caret.right(text, keys.shift);
    }
    if keys.home {
        caret.home(keys.shift);
    }
    if keys.end {
        caret.end(text, keys.shift);
    }
    if keys.select_all {
        caret.select_all(text);
    }
}

thread_local! {
    static DISPLAY_CARET: std::cell::Cell<Caret> = const { std::cell::Cell::new(Caret { pos: usize::MAX, anchor: None }) };
}

/// The caret of the text field being typed into, for the places that draw its text (the app sets it every frame).
pub fn set_display_caret(caret: Caret) {
    DISPLAY_CARET.with(|cell| cell.set(caret));
}

/// `text` with the caret bar after `pos` characters.
#[must_use]
pub fn with_caret_at(text: &str, pos: usize) -> String {
    let at = pos.min(text.chars().count());
    let mut out: String = text.chars().take(at).collect();
    out.push('|');
    out.extend(text.chars().skip(at));
    out
}

/// `text` with the caret bar where the caret is (at the end when none was set).
#[must_use]
pub fn with_caret(text: &str) -> String {
    let caret = DISPLAY_CARET.with(std::cell::Cell::get);
    let at = caret.pos.min(text.chars().count());
    let mut out: String = text.chars().take(at).collect();
    out.push('|');
    out.extend(text.chars().skip(at));
    out
}

/// The character boundary nearest to `x` in a text whose boundaries sit at `edges` (`edges[i]` is the distance from the text's start to before character
/// `i`, with the end last).
#[must_use]
pub fn nearest_edge(edges: &[f32], x: f32) -> usize {
    edges
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - x).abs().partial_cmp(&(b.1 - x).abs()).unwrap_or(std::cmp::Ordering::Equal))
        .map_or(0, |(index, _)| index)
}

#[cfg(test)]
mod tests {
    #[test]
    fn edit_types_at_the_caret_moves_it_and_honours_the_limit() {
        let mut text = String::from("abc");
        let mut caret = Caret::at_end(&text);
        edit(&mut text, &mut caret, "", 5, EditKeys { left: true, ..EditKeys::default() });
        edit(&mut text, &mut caret, "XY", 5, EditKeys::default());
        assert_eq!(text, "abXYc");
        edit(&mut text, &mut caret, "Z", 5, EditKeys::default());
        assert_eq!(text, "abXYc", "full");
        edit(&mut text, &mut caret, "", 5, EditKeys { backspace: true, home: true, ..EditKeys::default() });
        assert_eq!(text, "abXc");
        assert_eq!(caret.pos, 0);
        set_display_caret(Caret { pos: 2, anchor: None });
        assert_eq!(with_caret(&text), "ab|Xc");
        set_display_caret(Caret { pos: usize::MAX, anchor: None });
        assert_eq!(with_caret("hi"), "hi|");
    }

    use super::*;

    fn field(text: &str) -> (String, Caret) {
        (text.to_owned(), Caret::at_end(text))
    }

    #[test]
    fn typing_goes_in_at_the_caret_and_replaces_a_selection() {
        let (mut text, mut caret) = field("abcd");
        caret.left(false);
        caret.left(false);
        caret.insert(&mut text, "XY");
        assert_eq!((text.as_str(), caret.pos), ("abXYcd", 4));
        caret.select_all(&text);
        caret.insert(&mut text, "é\n");
        assert_eq!((text.as_str(), caret.pos), ("é", 1));
        // Positions count characters: the next character goes after the multi-byte one.
        caret.insert(&mut text, "z");
        assert_eq!(text, "éz");
    }

    #[test]
    fn backspace_and_delete_remove_beside_the_caret_or_the_selection() {
        let (mut text, mut caret) = field("hello");
        caret.home(false);
        caret.backspace(&mut text);
        assert_eq!(text, "hello");
        caret.delete(&mut text);
        assert_eq!(text, "ello");
        caret.end(&text, false);
        caret.backspace(&mut text);
        assert_eq!((text.as_str(), caret.pos), ("ell", 3));
        caret.delete(&mut text);
        assert_eq!(text, "ell");
        caret.left(true);
        caret.left(true);
        assert_eq!(caret.selection(), Some((1, 3)));
        caret.delete(&mut text);
        assert_eq!((text.as_str(), caret.pos, caret.selection()), ("e", 1, None));
    }

    #[test]
    fn arrows_move_shift_extends_and_a_plain_arrow_collapses_the_selection() {
        let (text, mut caret) = field("abcdef");
        caret.home(false);
        caret.right(&text, true);
        caret.right(&text, true);
        assert_eq!(caret.selection(), Some((0, 2)));
        caret.right(&text, false);
        assert_eq!((caret.pos, caret.selection()), (2, None));
        caret.end(&text, true);
        assert_eq!(caret.selection(), Some((2, 6)));
        caret.left(false);
        assert_eq!((caret.pos, caret.selection()), (2, None));
        caret.home(false);
        caret.left(false);
        assert_eq!(caret.pos, 0);
    }

    #[test]
    fn a_press_places_the_caret_and_a_drag_selects() {
        let edges = [0.0, 10.0, 20.0, 30.0];
        assert_eq!(nearest_edge(&edges, 14.0), 1);
        assert_eq!(nearest_edge(&edges, 99.0), 3);
        let mut caret = Caret::default();
        caret.press(nearest_edge(&edges, 11.0), false);
        caret.drag(nearest_edge(&edges, 29.0));
        assert_eq!(caret.selection(), Some((1, 3)));
        caret.press(0, true);
        assert_eq!(caret.selection(), Some((0, 1)));
    }
}
