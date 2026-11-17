//! A text with a cursor and a selection, as the console keeps the SQL it
//! shows: the same names as the editor's own (`Content`, `Action`,
//! `Motion`), so its logic reads the same. The editor of the panel holds
//! what is typed; this follows it (its text, and its cursor when asked).
//! The motions and edits the editor's panel does itself are kept for the
//! tests of the console.
#![cfg_attr(not(test), allow(dead_code))]

use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub position: Position,
    /// The other end of the selection, if any.
    pub selection: Option<Position>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    Home,
    DocumentEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Insert(char),
    Paste(Arc<String>),
    Backspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    Select(Motion),
    SelectAll,
    Edit(Edit),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Content {
    lines: Vec<String>,
    cursor: Position,
    anchor: Option<Position>,
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

fn byte_at(s: &str, column: usize) -> usize {
    s.char_indices().nth(column).map_or(s.len(), |(i, _)| i)
}

impl Content {
    pub fn new() -> Self {
        Self::with_text("")
    }

    pub fn with_text(text: &str) -> Self {
        Content {
            lines: text.split('\n').map(str::to_string).collect(),
            cursor: Position::default(),
            anchor: None,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn cursor(&self) -> Cursor {
        Cursor {
            position: self.cursor,
            selection: self.anchor,
        }
    }

    /// Puts the cursor (and the other end of a selection) there.
    pub fn move_to(&mut self, cursor: Cursor) {
        self.cursor = self.clamp(cursor.position);
        self.anchor = cursor.selection.map(|p| self.clamp(p));
    }

    fn clamp(&self, p: Position) -> Position {
        let line = p.line.min(self.lines.len().saturating_sub(1));
        Position {
            line,
            column: p.column.min(chars(&self.lines[line])),
        }
    }

    fn ordered(&self) -> Option<(Position, Position)> {
        let anchor = self.anchor?;
        let key = |p: Position| (p.line, p.column);
        if key(anchor) == key(self.cursor) {
            return None;
        }
        Some(if key(anchor) < key(self.cursor) {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        })
    }

    pub fn selection(&self) -> Option<String> {
        let (a, b) = self.ordered()?;
        let mut out = String::new();
        for line in a.line..=b.line {
            let text = &self.lines[line];
            let from = if line == a.line { a.column } else { 0 };
            let to = if line == b.line {
                b.column
            } else {
                chars(text)
            };
            out.push_str(&text[byte_at(text, from)..byte_at(text, to)]);
            if line != b.line {
                out.push('\n');
            }
        }
        Some(out)
    }

    fn moved(&self, motion: Motion) -> Position {
        let p = self.cursor;
        let last = self.lines.len() - 1;
        let len = |l: usize| chars(&self.lines[l]);
        match motion {
            Motion::Left if p.column > 0 => Position {
                column: p.column - 1,
                ..p
            },
            Motion::Left if p.line > 0 => Position {
                line: p.line - 1,
                column: len(p.line - 1),
            },
            Motion::Left => p,
            Motion::Right if p.column < len(p.line) => Position {
                column: p.column + 1,
                ..p
            },
            Motion::Right if p.line < last => Position {
                line: p.line + 1,
                column: 0,
            },
            Motion::Right => p,
            Motion::Up if p.line > 0 => Position {
                line: p.line - 1,
                column: p.column.min(len(p.line - 1)),
            },
            Motion::Up => Position { line: 0, column: 0 },
            Motion::Down if p.line < last => Position {
                line: p.line + 1,
                column: p.column.min(len(p.line + 1)),
            },
            Motion::Down => Position {
                line: last,
                column: len(last),
            },
            Motion::Home => Position { column: 0, ..p },
            Motion::DocumentEnd => Position {
                line: last,
                column: len(last),
            },
        }
    }

    /// Takes out what is selected; the cursor goes where it was.
    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.ordered() else {
            self.anchor = None;
            return false;
        };
        let head = self.lines[a.line][..byte_at(&self.lines[a.line], a.column)].to_string();
        let tail = self.lines[b.line][byte_at(&self.lines[b.line], b.column)..].to_string();
        self.lines
            .splice(a.line..=b.line, [format!("{head}{tail}")]);
        self.cursor = a;
        self.anchor = None;
        true
    }

    fn insert(&mut self, text: &str) {
        self.delete_selection();
        let p = self.cursor;
        let line = &self.lines[p.line];
        let at = byte_at(line, p.column);
        let (head, tail) = (line[..at].to_string(), line[at..].to_string());
        let mut parts: Vec<String> = text.split('\n').map(str::to_string).collect();
        let last = parts.len() - 1;
        let column = if last == 0 {
            p.column + chars(&parts[0])
        } else {
            chars(&parts[last])
        };
        parts[0] = format!("{head}{}", parts[0]);
        parts[last].push_str(&tail);
        let n = parts.len();
        self.lines.splice(p.line..=p.line, parts);
        self.cursor = Position {
            line: p.line + n - 1,
            column,
        };
    }

    pub fn perform(&mut self, action: Action) {
        match action {
            Action::Move(motion) => {
                self.cursor = self.moved(motion);
                self.anchor = None;
            }
            Action::Select(motion) => {
                if self.anchor.is_none() {
                    self.anchor = Some(self.cursor);
                }
                self.cursor = self.moved(motion);
            }
            Action::SelectAll => {
                self.anchor = Some(Position::default());
                self.cursor = self.moved(Motion::DocumentEnd);
            }
            Action::Edit(Edit::Insert(c)) => self.insert(&c.to_string()),
            Action::Edit(Edit::Paste(text)) => self.insert(&text),
            Action::Edit(Edit::Backspace) => {
                if !self.delete_selection() {
                    self.anchor = Some(self.moved(Motion::Left));
                    self.delete_selection();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_moving_and_selecting_follow_the_text() {
        let mut c = Content::with_text("ab\ncd");
        c.perform(Action::Move(Motion::DocumentEnd));
        assert_eq!(c.cursor().position, Position { line: 1, column: 2 });
        c.perform(Action::Edit(Edit::Paste(Arc::new("e\nfg".into()))));
        assert_eq!(c.text(), "ab\ncde\nfg");
        assert_eq!(c.cursor().position, Position { line: 2, column: 2 });
        c.perform(Action::Move(Motion::Up));
        c.perform(Action::Select(Motion::Home));
        assert_eq!(c.selection().as_deref(), Some("cd"));
        c.perform(Action::Edit(Edit::Backspace));
        assert_eq!(c.text(), "ab\ne\nfg");
        c.perform(Action::SelectAll);
        assert_eq!(c.selection().as_deref(), Some("ab\ne\nfg"));
        c.perform(Action::Edit(Edit::Insert('é')));
        assert_eq!(c.text(), "é");
        c.move_to(Cursor {
            position: Position { line: 9, column: 9 },
            selection: None,
        });
        assert_eq!(c.cursor().position, Position { line: 0, column: 1 });
    }
}
