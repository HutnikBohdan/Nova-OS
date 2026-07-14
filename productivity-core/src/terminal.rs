pub const COLS: usize = 100;
pub const ROWS: usize = 30;
pub const SCROLLBACK: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightWhite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
}
impl Cell {
    const fn blank() -> Self {
        Self {
            ch: ' ',
            foreground: Color::Default,
            background: Color::Default,
            bold: false,
        }
    }
}

#[derive(Clone, Copy)]
enum Parser {
    Text,
    Escape,
    Csi {
        params: [u16; 4],
        count: usize,
        value: u16,
    },
}

pub struct Terminal {
    cells: [[Cell; COLS]; SCROLLBACK],
    head: usize,
    lines: usize,
    row: usize,
    col: usize,
    foreground: Color,
    background: Color,
    bold: bool,
    parser: Parser,
}

impl Terminal {
    pub const fn new() -> Self {
        Self {
            cells: [[Cell::blank(); COLS]; SCROLLBACK],
            head: 0,
            lines: 1,
            row: 0,
            col: 0,
            foreground: Color::Default,
            background: Color::Default,
            bold: false,
            parser: Parser::Text,
        }
    }

    pub fn write_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.feed(ch);
        }
    }
    pub const fn line_count(&self) -> usize {
        self.lines
    }
    pub const fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }
    pub fn cell(&self, line_from_oldest: usize, col: usize) -> Option<Cell> {
        if line_from_oldest >= self.lines || col >= COLS {
            return None;
        }
        let oldest = (self.head + SCROLLBACK + 1 - self.lines) % SCROLLBACK;
        Some(self.cells[(oldest + line_from_oldest) % SCROLLBACK][col])
    }
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    fn feed(&mut self, ch: char) {
        match self.parser {
            Parser::Text if ch == '\x1b' => self.parser = Parser::Escape,
            Parser::Text => self.put(ch),
            Parser::Escape if ch == '[' => {
                self.parser = Parser::Csi {
                    params: [0; 4],
                    count: 0,
                    value: 0,
                }
            }
            Parser::Escape => {
                self.parser = Parser::Text;
                self.put(ch);
            }
            Parser::Csi {
                mut params,
                mut count,
                mut value,
            } => {
                if let Some(d) = ch.to_digit(10) {
                    value = value.saturating_mul(10).saturating_add(d as u16);
                    self.parser = Parser::Csi {
                        params,
                        count,
                        value,
                    };
                } else if ch == ';' {
                    if count < 4 {
                        params[count] = value;
                        count += 1;
                    }
                    self.parser = Parser::Csi {
                        params,
                        count,
                        value: 0,
                    };
                } else {
                    if count < 4 {
                        params[count] = value;
                        count += 1;
                    }
                    self.apply_csi(ch, &params[..count]);
                    self.parser = Parser::Text;
                }
            }
        }
    }

    fn put(&mut self, ch: char) {
        match ch {
            '\n' => self.newline(),
            '\r' => self.col = 0,
            '\x08' => self.col = self.col.saturating_sub(1),
            '\t' => {
                let target = ((self.col / 4) + 1) * 4;
                while self.col < target.min(COLS) {
                    self.put(' ');
                }
            }
            c if !c.is_control() => {
                self.cells[self.head][self.col] = Cell {
                    ch: c,
                    foreground: self.foreground,
                    background: self.background,
                    bold: self.bold,
                };
                self.col += 1;
                if self.col == COLS {
                    self.newline();
                }
            }
            _ => {}
        }
    }
    fn newline(&mut self) {
        self.head = (self.head + 1) % SCROLLBACK;
        self.cells[self.head] = [Cell::blank(); COLS];
        self.lines = (self.lines + 1).min(SCROLLBACK);
        self.row = self.row.saturating_add(1);
        self.col = 0;
    }
    fn apply_csi(&mut self, command: char, params: &[u16]) {
        match command {
            'm' => {
                for p in params {
                    match *p {
                        0 => {
                            self.foreground = Color::Default;
                            self.background = Color::Default;
                            self.bold = false;
                        }
                        1 => self.bold = true,
                        30..=37 => self.foreground = ansi_color(*p - 30),
                        40..=47 => self.background = ansi_color(*p - 40),
                        90 => self.foreground = Color::BrightBlack,
                        97 => self.foreground = Color::BrightWhite,
                        _ => {}
                    }
                }
            }
            'J' if params.first().copied().unwrap_or(0) == 2 => self.clear(),
            'K' => {
                for cell in &mut self.cells[self.head][self.col..] {
                    *cell = Cell::blank();
                }
            }
            'A' => {
                self.row = self
                    .row
                    .saturating_sub(params.first().copied().unwrap_or(1).max(1) as usize)
            }
            'C' => {
                self.col =
                    (self.col + params.first().copied().unwrap_or(1).max(1) as usize).min(COLS - 1)
            }
            'D' => {
                self.col = self
                    .col
                    .saturating_sub(params.first().copied().unwrap_or(1).max(1) as usize)
            }
            _ => {}
        }
    }
}

const fn ansi_color(index: u16) -> Color {
    match index {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::White,
    }
}
impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_and_ansi() {
        let mut t = Terminal::new();
        t.write_str("Nova \x1b[1;32mготова\x1b[0m");
        assert_eq!(t.cell(0, 5).unwrap().ch, 'г');
        assert_eq!(t.cell(0, 5).unwrap().foreground, Color::Green);
        assert!(t.cell(0, 5).unwrap().bold);
    }
    #[test]
    fn scrolling_is_bounded() {
        let mut t = Terminal::new();
        for _ in 0..150 {
            t.write_str("рядок\n");
        }
        assert_eq!(t.line_count(), SCROLLBACK);
    }
}
