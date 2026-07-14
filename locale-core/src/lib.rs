#![no_std]
#![forbid(unsafe_code)]

//! Allocation-free Ukrainian locale, input and accessibility policy for Nova OS.

use core::{
    cmp::Ordering,
    fmt::{self, Write},
};

pub const LOCALE: &str = "uk-UA";

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FixedString<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> FixedString<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
    pub fn try_from_str(value: &str) -> Result<Self, TextError> {
        let mut result = Self::new();
        result.push_str(value)?;
        Ok(result)
    }
    pub fn push_str(&mut self, value: &str) -> Result<(), TextError> {
        if self.len + value.len() > N {
            return Err(TextError::Full);
        }
        self.bytes[self.len..self.len + value.len()].copy_from_slice(value.as_bytes());
        self.len += value.len();
        Ok(())
    }
    pub fn push(&mut self, value: char) -> Result<(), TextError> {
        let mut encoded = [0; 4];
        self.push_str(value.encode_utf8(&mut encoded))
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl<const N: usize> Default for FixedString<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize> fmt::Debug for FixedString<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}
impl<const N: usize> Write for FixedString<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s).map_err(|_| fmt::Error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextError {
    Full,
    MissingArgument,
    InvalidDate,
    InvalidTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plural {
    One,
    Few,
    Many,
    Other,
}

/// CLDR-compatible integer cardinal categories for Ukrainian.
pub const fn ukrainian_plural(value: i64) -> Plural {
    let n = value.unsigned_abs();
    let mod10 = n % 10;
    let mod100 = n % 100;
    if mod10 == 1 && mod100 != 11 {
        Plural::One
    } else if mod10 >= 2 && mod10 <= 4 && !(mod100 >= 12 && mod100 <= 14) {
        Plural::Few
    } else {
        Plural::Many
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageId {
    Welcome,
    Search,
    Settings,
    Files,
    Browser,
    Terminal,
    AiAssistant,
    Install,
    Cancel,
    Close,
    Save,
    Open,
    Delete,
    Retry,
    PowerOff,
    Restart,
    FileCount,
    WindowCount,
    DownloadCount,
    NotificationCount,
    PermissionQuestion,
    UnsavedChanges,
    NetworkOffline,
    UpdateReady,
}

pub const fn message(id: MessageId, plural: Plural) -> &'static str {
    match id {
        MessageId::Welcome => "Ласкаво просимо до Nova",
        MessageId::Search => "Пошук",
        MessageId::Settings => "Налаштування",
        MessageId::Files => "Файли",
        MessageId::Browser => "Браузер",
        MessageId::Terminal => "Термінал",
        MessageId::AiAssistant => "Помічник Nova",
        MessageId::Install => "Установити",
        MessageId::Cancel => "Скасувати",
        MessageId::Close => "Закрити",
        MessageId::Save => "Зберегти",
        MessageId::Open => "Відкрити",
        MessageId::Delete => "Видалити",
        MessageId::Retry => "Повторити",
        MessageId::PowerOff => "Вимкнути",
        MessageId::Restart => "Перезапустити",
        MessageId::FileCount => match plural {
            Plural::One => "{0} файл",
            Plural::Few => "{0} файли",
            _ => "{0} файлів",
        },
        MessageId::WindowCount => match plural {
            Plural::One => "{0} вікно",
            Plural::Few => "{0} вікна",
            _ => "{0} вікон",
        },
        MessageId::DownloadCount => match plural {
            Plural::One => "{0} завантаження",
            Plural::Few => "{0} завантаження",
            _ => "{0} завантажень",
        },
        MessageId::NotificationCount => match plural {
            Plural::One => "{0} сповіщення",
            Plural::Few => "{0} сповіщення",
            _ => "{0} сповіщень",
        },
        MessageId::PermissionQuestion => "Дозволити програмі «{0}» дію «{1}»?",
        MessageId::UnsavedChanges => "У документі «{0}» є незбережені зміни.",
        MessageId::NetworkOffline => "Немає з’єднання з мережею",
        MessageId::UpdateReady => "Оновлення готове до встановлення",
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FormatArg<'a> {
    Str(&'a str),
    Integer(i64),
}

pub fn format_message<const N: usize>(
    id: MessageId,
    count: Option<i64>,
    args: &[FormatArg<'_>],
) -> Result<FixedString<N>, TextError> {
    let template = message(id, count.map_or(Plural::Other, ukrainian_plural));
    let mut out = FixedString::new();
    let bytes = template.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'{' && pos + 2 < bytes.len() && bytes[pos + 2] == b'}' {
            let index = bytes[pos + 1].wrapping_sub(b'0') as usize;
            let arg = args.get(index).ok_or(TextError::MissingArgument)?;
            match arg {
                FormatArg::Str(value) => out.push_str(value)?,
                FormatArg::Integer(value) => {
                    write!(&mut out, "{value}").map_err(|_| TextError::Full)?
                }
            }
            pos += 3;
        } else {
            let ch = template[pos..].chars().next().ok_or(TextError::Full)?;
            out.push(ch)?;
            pos += ch.len_utf8();
        }
    }
    Ok(out)
}

pub mod format {
    use super::{FixedString, TextError};
    use core::fmt::Write;

    const MONTHS: [&str; 12] = [
        "січня",
        "лютого",
        "березня",
        "квітня",
        "травня",
        "червня",
        "липня",
        "серпня",
        "вересня",
        "жовтня",
        "листопада",
        "грудня",
    ];
    const WEEKDAYS: [&str; 7] = [
        "понеділок",
        "вівторок",
        "середа",
        "четвер",
        "п’ятниця",
        "субота",
        "неділя",
    ];

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Date {
        pub year: u16,
        pub month: u8,
        pub day: u8,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Time {
        pub hour: u8,
        pub minute: u8,
        pub second: u8,
    }

    impl Date {
        pub const fn is_valid(self) -> bool {
            if self.month < 1 || self.month > 12 || self.day < 1 {
                return false;
            }
            let leap = self.year.is_multiple_of(4)
                && (!self.year.is_multiple_of(100) || self.year.is_multiple_of(400));
            let days = match self.month {
                2 if leap => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            self.day <= days
        }
    }
    impl Time {
        pub const fn is_valid(self) -> bool {
            self.hour < 24 && self.minute < 60 && self.second < 60
        }
    }

    pub fn date_long<const N: usize>(date: Date) -> Result<FixedString<N>, TextError> {
        if !date.is_valid() {
            return Err(TextError::InvalidDate);
        }
        let mut out = FixedString::new();
        write!(
            &mut out,
            "{} {} {} року",
            date.day,
            MONTHS[date.month as usize - 1],
            date.year
        )
        .map_err(|_| TextError::Full)?;
        Ok(out)
    }
    pub fn date_numeric<const N: usize>(date: Date) -> Result<FixedString<N>, TextError> {
        if !date.is_valid() {
            return Err(TextError::InvalidDate);
        }
        let mut out = FixedString::new();
        write!(
            &mut out,
            "{:02}.{:02}.{:04}",
            date.day, date.month, date.year
        )
        .map_err(|_| TextError::Full)?;
        Ok(out)
    }
    pub fn time<const N: usize>(time: Time, seconds: bool) -> Result<FixedString<N>, TextError> {
        if !time.is_valid() {
            return Err(TextError::InvalidTime);
        }
        let mut out = FixedString::new();
        if seconds {
            write!(
                &mut out,
                "{:02}:{:02}:{:02}",
                time.hour, time.minute, time.second
            )
        } else {
            write!(&mut out, "{:02}:{:02}", time.hour, time.minute)
        }
        .map_err(|_| TextError::Full)?;
        Ok(out)
    }
    pub const fn weekday_name(monday_zero: u8) -> Option<&'static str> {
        if monday_zero < 7 {
            Some(WEEKDAYS[monday_zero as usize])
        } else {
            None
        }
    }

    pub fn integer<const N: usize>(value: i64) -> Result<FixedString<N>, TextError> {
        let mut raw = FixedString::<32>::new();
        write!(&mut raw, "{}", value.unsigned_abs()).map_err(|_| TextError::Full)?;
        let mut out = FixedString::new();
        if value < 0 {
            out.push('−')?;
        }
        let len = raw.len();
        for (i, ch) in raw.as_str().chars().enumerate() {
            if i != 0 && (len - i).is_multiple_of(3) {
                out.push('\u{00a0}')?;
            }
            out.push(ch)?;
        }
        Ok(out)
    }

    /// Formats an integer scaled by `10^scale`, using the Ukrainian decimal comma.
    pub fn decimal<const N: usize>(scaled: i64, scale: u8) -> Result<FixedString<N>, TextError> {
        if scale == 0 {
            return integer(scaled);
        }
        let divisor = 10_u64.checked_pow(scale as u32).ok_or(TextError::Full)?;
        let abs = scaled.unsigned_abs();
        let whole = (abs / divisor) as i64;
        let mut out = integer::<N>(if scaled < 0 { -whole } else { whole })?;
        if scaled < 0 && abs < divisor {
            out.clear();
            out.push('−')?;
            out.push('0')?;
        }
        out.push(',')?;
        write!(
            &mut out,
            "{:0width$}",
            abs % divisor,
            width = scale as usize
        )
        .map_err(|_| TextError::Full)?;
        Ok(out)
    }
}

fn fold_char(value: char) -> Option<char> {
    Some(match value {
        'А'..='Я' => char::from_u32(value as u32 + 32)?,
        'Ґ' => 'ґ',
        'Є' => 'є',
        'І' => 'і',
        'Ї' => 'ї',
        '\u{2019}' | '\u{2018}' | '\u{02bc}' | '`' => '\'',
        '\u{0301}' => return None,
        _ => value.to_ascii_lowercase(),
    })
}

pub fn search_equal(left: &str, right: &str) -> bool {
    let mut a = left.chars().filter_map(fold_char);
    let mut b = right.chars().filter_map(fold_char);
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => {}
            _ => return false,
        }
    }
}

pub fn search_contains(haystack: &str, needle: &str) -> bool {
    let mut wanted = ['\0'; 64];
    let mut wanted_len = 0;
    for ch in needle.chars().filter_map(fold_char) {
        if wanted_len == wanted.len() {
            return false;
        }
        wanted[wanted_len] = ch;
        wanted_len += 1;
    }
    if wanted_len == 0 {
        return true;
    }
    let mut window = ['\0'; 64];
    let mut seen = 0usize;
    for ch in haystack.chars().filter_map(fold_char) {
        window[seen % wanted_len] = ch;
        seen += 1;
        if seen >= wanted_len {
            let start = seen % wanted_len;
            if (0..wanted_len).all(|i| window[(start + i) % wanted_len] == wanted[i]) {
                return true;
            }
        }
    }
    false
}

fn collation_weight(ch: char) -> (u8, u32) {
    const ALPHABET: &str = "а б в г ґ д е є ж з и і ї й к л м н о п р с т у ф х ц ч ш щ ь ю я";
    let folded = fold_char(ch).unwrap_or(ch);
    for (index, candidate) in ALPHABET.chars().filter(|c| *c != ' ').enumerate() {
        if folded == candidate {
            return (0, index as u32);
        }
    }
    (1, folded as u32)
}

pub fn collate(left: &str, right: &str) -> Ordering {
    let mut a = left.chars().filter_map(fold_char);
    let mut b = right.chars().filter_map(fold_char);
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) => match collation_weight(x).cmp(&collation_weight(y)) {
                Ordering::Equal => {}
                order => return order,
            },
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

pub mod keyboard {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Layout {
        Ukrainian,
        EnglishUs,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Key {
        A,
        B,
        C,
        D,
        E,
        F,
        G,
        H,
        I,
        J,
        K,
        L,
        M,
        N,
        O,
        P,
        Q,
        R,
        S,
        T,
        U,
        V,
        W,
        X,
        Y,
        Z,
        Digit(u8),
        Minus,
        Equal,
        LeftBracket,
        RightBracket,
        Semicolon,
        Quote,
        Comma,
        Period,
        Slash,
        Backslash,
        Space,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Modifiers {
        pub shift: bool,
        pub alt_gr: bool,
        pub caps_lock: bool,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum KeyOutput {
        Character(char),
        Dead(DeadKey),
        None,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DeadKey {
        Acute,
        Grave,
        Diaeresis,
    }

    fn letter_index(key: Key) -> Option<usize> {
        match key {
            Key::A => Some(0),
            Key::B => Some(1),
            Key::C => Some(2),
            Key::D => Some(3),
            Key::E => Some(4),
            Key::F => Some(5),
            Key::G => Some(6),
            Key::H => Some(7),
            Key::I => Some(8),
            Key::J => Some(9),
            Key::K => Some(10),
            Key::L => Some(11),
            Key::M => Some(12),
            Key::N => Some(13),
            Key::O => Some(14),
            Key::P => Some(15),
            Key::Q => Some(16),
            Key::R => Some(17),
            Key::S => Some(18),
            Key::T => Some(19),
            Key::U => Some(20),
            Key::V => Some(21),
            Key::W => Some(22),
            Key::X => Some(23),
            Key::Y => Some(24),
            Key::Z => Some(25),
            _ => None,
        }
    }

    pub fn translate(layout: Layout, key: Key, mods: Modifiers) -> KeyOutput {
        if mods.alt_gr && key == Key::G && layout == Layout::Ukrainian {
            return KeyOutput::Character(if mods.shift { 'Ґ' } else { 'ґ' });
        }
        if mods.alt_gr && key == Key::Quote {
            return KeyOutput::Dead(DeadKey::Acute);
        }
        let upper = mods.shift ^ mods.caps_lock;
        if let Some(index) = letter_index(key) {
            let lower = match layout {
                Layout::EnglishUs => "abcdefghijklmnopqrstuvwxyz".chars().nth(index),
                // Physical US positions: Ukrainian ЙЦУКЕН layout.
                Layout::Ukrainian => "фисвуапршолдьтщзйкіегмцчня".chars().nth(index),
            };
            if let Some(ch) = lower {
                return KeyOutput::Character(if upper {
                    ch.to_uppercase().next().unwrap_or(ch)
                } else {
                    ch
                });
            }
        }
        let ch = match key {
            Key::Digit(n) if n < 10 => {
                if mods.shift {
                    ")!\"№;%:?*(".chars().nth(n as usize)
                } else {
                    char::from_digit(n as u32, 10)
                }
            }
            Key::Space => Some(' '),
            Key::Minus => Some(if mods.shift { '_' } else { '-' }),
            Key::Equal => Some(if mods.shift { '+' } else { '=' }),
            Key::LeftBracket => Some(if layout == Layout::Ukrainian {
                if mods.shift { 'Х' } else { 'х' }
            } else if mods.shift {
                '{'
            } else {
                '['
            }),
            Key::RightBracket => Some(if layout == Layout::Ukrainian {
                if mods.shift { 'Ї' } else { 'ї' }
            } else if mods.shift {
                '}'
            } else {
                ']'
            }),
            Key::Semicolon => Some(if layout == Layout::Ukrainian {
                if mods.shift { 'Ж' } else { 'ж' }
            } else if mods.shift {
                ':'
            } else {
                ';'
            }),
            Key::Quote => Some(if layout == Layout::Ukrainian {
                if mods.shift { 'Є' } else { 'є' }
            } else if mods.shift {
                '\"'
            } else {
                '\''
            }),
            Key::Comma => Some(if layout == Layout::Ukrainian {
                if mods.shift { 'Б' } else { 'б' }
            } else if mods.shift {
                '<'
            } else {
                ','
            }),
            Key::Period => Some(if layout == Layout::Ukrainian {
                if mods.shift { 'Ю' } else { 'ю' }
            } else if mods.shift {
                '>'
            } else {
                '.'
            }),
            Key::Slash => Some(if layout == Layout::Ukrainian {
                if mods.shift { ',' } else { '.' }
            } else if mods.shift {
                '?'
            } else {
                '/'
            }),
            Key::Backslash => Some(if mods.shift { '/' } else { '\\' }),
            _ => None,
        };
        ch.map_or(KeyOutput::None, KeyOutput::Character)
    }

    #[derive(Debug, Default, Clone, Copy)]
    pub struct ComposeState {
        dead: Option<DeadKey>,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Composed {
        pub first: char,
        pub second: Option<char>,
    }
    impl ComposeState {
        pub const fn new() -> Self {
            Self { dead: None }
        }
        pub fn feed(&mut self, output: KeyOutput) -> Option<Composed> {
            match output {
                KeyOutput::Dead(dead) => {
                    self.dead = Some(dead);
                    None
                }
                KeyOutput::None => None,
                KeyOutput::Character(ch) => {
                    let Some(dead) = self.dead.take() else {
                        return Some(Composed {
                            first: ch,
                            second: None,
                        });
                    };
                    let combined = match (dead, ch) {
                        (DeadKey::Diaeresis, 'і') => Some('ї'),
                        (DeadKey::Diaeresis, 'І') => Some('Ї'),
                        (DeadKey::Acute, 'e') => Some('é'),
                        (DeadKey::Acute, 'E') => Some('É'),
                        (DeadKey::Grave, 'e') => Some('è'),
                        (DeadKey::Grave, 'E') => Some('È'),
                        _ => None,
                    };
                    combined.map_or_else(
                        || {
                            Some(Composed {
                                first: match dead {
                                    DeadKey::Acute => '´',
                                    DeadKey::Grave => '`',
                                    DeadKey::Diaeresis => '¨',
                                },
                                second: Some(ch),
                            })
                        },
                        |first| {
                            Some(Composed {
                                first,
                                second: None,
                            })
                        },
                    )
                }
            }
        }
    }
}

pub mod accessibility {
    use super::{FixedString, TextError};

    pub const MAX_NODES: usize = 64;
    pub const MAX_UTTERANCES: usize = 16;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Role {
        Application,
        Window,
        Dialog,
        Group,
        Button,
        CheckBox,
        RadioButton,
        TextField,
        SearchBox,
        Link,
        Image,
        Heading,
        List,
        ListItem,
        Menu,
        MenuItem,
        Slider,
        ProgressBar,
        Status,
        Alert,
        Document,
    }
    impl Role {
        pub const fn name_uk(self) -> &'static str {
            match self {
                Self::Application => "програма",
                Self::Window => "вікно",
                Self::Dialog => "діалог",
                Self::Group => "група",
                Self::Button => "кнопка",
                Self::CheckBox => "прапорець",
                Self::RadioButton => "перемикач",
                Self::TextField => "текстове поле",
                Self::SearchBox => "поле пошуку",
                Self::Link => "посилання",
                Self::Image => "зображення",
                Self::Heading => "заголовок",
                Self::List => "список",
                Self::ListItem => "елемент списку",
                Self::Menu => "меню",
                Self::MenuItem => "пункт меню",
                Self::Slider => "повзунок",
                Self::ProgressBar => "індикатор виконання",
                Self::Status => "стан",
                Self::Alert => "попередження",
                Self::Document => "документ",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Actions(pub u16);
    impl Actions {
        pub const FOCUS: Self = Self(1);
        pub const ACTIVATE: Self = Self(2);
        pub const INCREMENT: Self = Self(4);
        pub const DECREMENT: Self = Self(8);
        pub const SET_VALUE: Self = Self(16);
        pub const DISMISS: Self = Self(32);
        pub const fn contains(self, other: Self) -> bool {
            self.0 & other.0 == other.0
        }
        pub const fn union(self, other: Self) -> Self {
            Self(self.0 | other.0)
        }
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct State {
        pub disabled: bool,
        pub selected: bool,
        pub checked: Option<bool>,
        pub expanded: Option<bool>,
        pub hidden: bool,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Live {
        Off,
        Polite,
        Assertive,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct NodeId(pub u32);
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Node {
        pub id: NodeId,
        pub parent: Option<NodeId>,
        pub role: Role,
        pub label: FixedString<96>,
        pub value: FixedString<96>,
        pub actions: Actions,
        pub state: State,
        pub live: Live,
    }
    impl Node {
        pub fn new(
            id: NodeId,
            parent: Option<NodeId>,
            role: Role,
            label: &str,
        ) -> Result<Self, TextError> {
            Ok(Self {
                id,
                parent,
                role,
                label: FixedString::try_from_str(label)?,
                value: FixedString::new(),
                actions: Actions::default(),
                state: State::default(),
                live: Live::Off,
            })
        }
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TreeError {
        Full,
        Duplicate,
        MissingParent,
        MissingNode,
        ActionUnavailable,
    }

    pub struct SemanticTree {
        nodes: [Option<Node>; MAX_NODES],
        focused: Option<NodeId>,
    }
    impl SemanticTree {
        pub const fn new() -> Self {
            Self {
                nodes: [None; MAX_NODES],
                focused: None,
            }
        }
        pub fn add(&mut self, node: Node) -> Result<(), TreeError> {
            if self.get(node.id).is_some() {
                return Err(TreeError::Duplicate);
            }
            if node.parent.is_some_and(|p| self.get(p).is_none()) {
                return Err(TreeError::MissingParent);
            }
            let slot = self
                .nodes
                .iter_mut()
                .find(|n| n.is_none())
                .ok_or(TreeError::Full)?;
            *slot = Some(node);
            Ok(())
        }
        pub fn remove(&mut self, id: NodeId) -> bool {
            let mut removed = false;
            for node in &mut self.nodes {
                if node.is_some_and(|n| n.id == id || n.parent == Some(id)) {
                    *node = None;
                    removed = true;
                }
            }
            if self.focused == Some(id) {
                self.focused = None;
            }
            removed
        }
        pub fn get(&self, id: NodeId) -> Option<&Node> {
            self.nodes.iter().flatten().find(|n| n.id == id)
        }
        pub fn focused(&self) -> Option<NodeId> {
            self.focused
        }
        pub fn focus(&mut self, id: NodeId) -> Result<(), TreeError> {
            let node = self.get(id).ok_or(TreeError::MissingNode)?;
            if node.state.hidden || node.state.disabled || !node.actions.contains(Actions::FOCUS) {
                return Err(TreeError::ActionUnavailable);
            }
            self.focused = Some(id);
            Ok(())
        }
        pub fn move_focus(&mut self, forward: bool) -> Option<NodeId> {
            let mut ids = [NodeId(0); MAX_NODES];
            let mut len = 0;
            for node in self.nodes.iter().flatten() {
                if !node.state.hidden
                    && !node.state.disabled
                    && node.actions.contains(Actions::FOCUS)
                {
                    ids[len] = node.id;
                    len += 1;
                }
            }
            if len == 0 {
                self.focused = None;
                return None;
            }
            let current = self
                .focused
                .and_then(|id| ids[..len].iter().position(|x| *x == id));
            let next = if forward {
                current.map_or(0, |i| (i + 1) % len)
            } else {
                current.map_or(len - 1, |i| (i + len - 1) % len)
            };
            self.focused = Some(ids[next]);
            self.focused
        }
        pub fn perform(&self, id: NodeId, action: Actions) -> Result<(), TreeError> {
            let node = self.get(id).ok_or(TreeError::MissingNode)?;
            if node.state.disabled || !node.actions.contains(action) {
                Err(TreeError::ActionUnavailable)
            } else {
                Ok(())
            }
        }
    }
    impl Default for SemanticTree {
        fn default() -> Self {
            Self::new()
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum Priority {
        Polite,
        Assertive,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Utterance {
        pub text: FixedString<192>,
        pub priority: Priority,
        pub interrupt: bool,
    }
    pub struct UtteranceQueue {
        items: [Option<Utterance>; MAX_UTTERANCES],
        head: usize,
        len: usize,
    }
    impl UtteranceQueue {
        pub const fn new() -> Self {
            Self {
                items: [None; MAX_UTTERANCES],
                head: 0,
                len: 0,
            }
        }
        pub fn announce(&mut self, text: &str, priority: Priority) -> Result<(), TextError> {
            let item = Utterance {
                text: FixedString::try_from_str(text)?,
                priority,
                interrupt: priority == Priority::Assertive,
            };
            if priority == Priority::Assertive {
                self.clear_polite();
            }
            if self.len == MAX_UTTERANCES {
                self.pop();
            }
            let tail = (self.head + self.len) % MAX_UTTERANCES;
            self.items[tail] = Some(item);
            self.len += 1;
            Ok(())
        }
        fn clear_polite(&mut self) {
            let mut kept = [None; MAX_UTTERANCES];
            let mut len = 0;
            while let Some(item) = self.pop() {
                if item.priority == Priority::Assertive {
                    kept[len] = Some(item);
                    len += 1;
                }
            }
            self.items = kept;
            self.head = 0;
            self.len = len;
        }
        pub fn pop(&mut self) -> Option<Utterance> {
            if self.len == 0 {
                return None;
            }
            let item = self.items[self.head].take();
            self.head = (self.head + 1) % MAX_UTTERANCES;
            self.len -= 1;
            item
        }
        pub const fn len(&self) -> usize {
            self.len
        }
        pub const fn is_empty(&self) -> bool {
            self.len == 0
        }
    }
    impl Default for UtteranceQueue {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contrast {
    Standard,
    High,
    Inverted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preferences {
    pub contrast: Contrast,
    pub reduced_motion: bool,
    pub screen_reader: bool,
    text_scale_percent: u16,
}
impl Preferences {
    pub const fn new() -> Self {
        Self {
            contrast: Contrast::Standard,
            reduced_motion: false,
            screen_reader: false,
            text_scale_percent: 100,
        }
    }
    pub fn set_text_scale(&mut self, percent: u16) {
        self.text_scale_percent = percent.clamp(75, 300);
    }
    pub const fn text_scale_percent(self) -> u16 {
        self.text_scale_percent
    }
    pub const fn animation_duration_ms(self, normal_ms: u32) -> u32 {
        if self.reduced_motion { 0 } else { normal_ms }
    }
}
impl Default for Preferences {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::accessibility::*;
    use super::keyboard::*;
    use super::*;

    #[test]
    fn plural_rules_cover_ukrainian_exceptions() {
        assert_eq!(ukrainian_plural(1), Plural::One);
        assert_eq!(ukrainian_plural(21), Plural::One);
        assert_eq!(ukrainian_plural(2), Plural::Few);
        assert_eq!(ukrainian_plural(14), Plural::Many);
        assert_eq!(ukrainian_plural(111), Plural::Many);
    }
    #[test]
    fn catalog_formats_plural_and_arguments() {
        let value = format_message::<32>(MessageId::FileCount, Some(24), &[FormatArg::Integer(24)])
            .unwrap();
        assert_eq!(value.as_str(), "24 файли");
        let question = format_message::<128>(
            MessageId::PermissionQuestion,
            None,
            &[FormatArg::Str("Браузер"), FormatArg::Str("відкрити камеру")],
        )
        .unwrap();
        assert!(question.as_str().contains("Браузер"));
    }
    #[test]
    fn locale_formats_date_time_and_number() {
        assert_eq!(
            format::date_long::<64>(format::Date {
                year: 2026,
                month: 7,
                day: 14
            })
            .unwrap()
            .as_str(),
            "14 липня 2026 року"
        );
        assert_eq!(
            format::time::<16>(
                format::Time {
                    hour: 9,
                    minute: 5,
                    second: 2
                },
                false
            )
            .unwrap()
            .as_str(),
            "09:05"
        );
        assert_eq!(
            format::integer::<32>(1_234_567).unwrap().as_str(),
            "1\u{a0}234\u{a0}567"
        );
        assert_eq!(format::decimal::<32>(-25, 1).unwrap().as_str(), "−2,5");
    }
    #[test]
    fn leap_date_validation() {
        assert!(
            format::Date {
                year: 2024,
                month: 2,
                day: 29
            }
            .is_valid()
        );
        assert!(
            !format::Date {
                year: 2026,
                month: 2,
                day: 29
            }
            .is_valid()
        );
    }
    #[test]
    fn search_fold_handles_case_apostrophe_and_stress() {
        assert!(search_equal("П’ЯТНИЦЯ", "п'ятниця"));
        assert!(search_equal("мо\u{301}ва", "мова"));
        assert!(search_contains("Налаштування мережі", "МЕРЕЖ"));
    }
    #[test]
    fn collation_uses_ukrainian_alphabet() {
        assert_eq!(collate("гуска", "ґанок"), Ordering::Less);
        assert_eq!(collate("и", "і"), Ordering::Less);
    }
    #[test]
    fn ukrainian_keyboard_and_altgr_ghe() {
        assert_eq!(
            translate(Layout::Ukrainian, Key::Q, Modifiers::default()),
            KeyOutput::Character('й')
        );
        assert_eq!(
            translate(
                Layout::Ukrainian,
                Key::G,
                Modifiers {
                    alt_gr: true,
                    ..Modifiers::default()
                }
            ),
            KeyOutput::Character('ґ')
        );
    }
    #[test]
    fn compose_dead_key() {
        let mut state = ComposeState::new();
        assert_eq!(state.feed(KeyOutput::Dead(DeadKey::Diaeresis)), None);
        assert_eq!(
            state.feed(KeyOutput::Character('і')),
            Some(Composed {
                first: 'ї',
                second: None
            })
        );
    }
    #[test]
    fn semantic_tree_focus_and_actions() {
        let mut tree = SemanticTree::new();
        let root = Node::new(NodeId(1), None, Role::Window, "Файли").unwrap();
        tree.add(root).unwrap();
        let mut button = Node::new(NodeId(2), Some(NodeId(1)), Role::Button, "Відкрити").unwrap();
        button.actions = Actions::FOCUS.union(Actions::ACTIVATE);
        tree.add(button).unwrap();
        assert_eq!(tree.move_focus(true), Some(NodeId(2)));
        assert_eq!(tree.perform(NodeId(2), Actions::ACTIVATE), Ok(()));
    }
    #[test]
    fn assertive_speech_discards_polite_backlog() {
        let mut queue = UtteranceQueue::new();
        queue.announce("завантаження", Priority::Polite).unwrap();
        queue.announce("помилка", Priority::Assertive).unwrap();
        assert_eq!(queue.len(), 1);
        let item = queue.pop().unwrap();
        assert_eq!(item.text.as_str(), "помилка");
        assert!(item.interrupt);
    }
    #[test]
    fn accessibility_preferences_are_bounded() {
        let mut prefs = Preferences::new();
        prefs.set_text_scale(999);
        assert_eq!(prefs.text_scale_percent(), 300);
        prefs.reduced_motion = true;
        assert_eq!(prefs.animation_duration_ms(250), 0);
    }
}
