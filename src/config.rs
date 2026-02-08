pub const DEFAULT_TITLE: &str = "QUBETX DEVELOPER TOOLS";
pub const DEFAULT_SUBTITLE: &str = "ND-300 NETWORK DIAGNOSTIC";

pub const MIN_LABEL_WIDTH: usize = 5;
pub const MAX_LABEL_WIDTH: usize = 16;
pub const MIN_DATA_WIDTH: usize = 20;
pub const MAX_DATA_WIDTH: usize = 40;
pub const BORDERS_PADDING: usize = 7;

pub mod chars {
    pub const TOP_LEFT: char = '┌';
    pub const TOP_RIGHT: char = '┐';
    pub const BOTTOM_LEFT: char = '└';
    pub const BOTTOM_RIGHT: char = '┘';
    pub const HORIZONTAL: char = '─';
    pub const VERTICAL: char = '│';
    pub const T_DOWN: char = '┬';
    pub const T_UP: char = '┴';
    pub const T_RIGHT: char = '├';
    pub const T_LEFT: char = '┤';
    pub const CROSS: char = '┼';
    pub const BAR_FILLED: char = '█';
    pub const BAR_EMPTY: char = '░';
}

pub mod ascii_chars {
    pub const TOP_LEFT: char = '+';
    pub const TOP_RIGHT: char = '+';
    pub const BOTTOM_LEFT: char = '+';
    pub const BOTTOM_RIGHT: char = '+';
    pub const HORIZONTAL: char = '-';
    pub const VERTICAL: char = '|';
    pub const T_DOWN: char = '+';
    pub const T_UP: char = '+';
    pub const T_RIGHT: char = '+';
    pub const T_LEFT: char = '+';
    pub const CROSS: char = '+';
    pub const BAR_FILLED: char = '#';
    pub const BAR_EMPTY: char = '.';
}

pub mod status_chars {
    pub const OK: &str = "\u{2713}";
    pub const WARN: &str = "\u{26A0}";
    pub const FAIL: &str = "\u{2717}";
    pub const SKIP: &str = "\u{2014}";

    pub const OK_ASCII: &str = "[OK]";
    pub const WARN_ASCII: &str = "[!!]";
    pub const FAIL_ASCII: &str = "[XX]";
    pub const SKIP_ASCII: &str = "[--]";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingMode {
    User,
    Technician,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub use_unicode: bool,
    pub use_colors: bool,
    pub title: Option<String>,
    pub mode: OperatingMode,
    pub format: OutputFormat,
    pub skip_speed: bool,
    pub speed_duration: u64,
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            use_unicode: true,
            use_colors: true,
            title: None,
            mode: OperatingMode::User,
            format: OutputFormat::Table,
            skip_speed: false,
            speed_duration: 10,
            verbose: false,
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(DEFAULT_TITLE)
    }

    pub fn subtitle(&self) -> &str {
        DEFAULT_SUBTITLE
    }

    pub fn box_chars(&self) -> BoxChars {
        if self.use_unicode {
            BoxChars::unicode()
        } else {
            BoxChars::ascii()
        }
    }

    pub fn bar_chars(&self) -> (char, char) {
        if self.use_unicode {
            (chars::BAR_FILLED, chars::BAR_EMPTY)
        } else {
            (ascii_chars::BAR_FILLED, ascii_chars::BAR_EMPTY)
        }
    }

    pub fn status_chars(&self, status: &crate::diagnostics::DiagnosticStatus) -> &'static str {
        use crate::diagnostics::DiagnosticStatus;
        if self.use_unicode {
            match status {
                DiagnosticStatus::Ok => status_chars::OK,
                DiagnosticStatus::Warn => status_chars::WARN,
                DiagnosticStatus::Fail => status_chars::FAIL,
                DiagnosticStatus::Skip => status_chars::SKIP,
            }
        } else {
            match status {
                DiagnosticStatus::Ok => status_chars::OK_ASCII,
                DiagnosticStatus::Warn => status_chars::WARN_ASCII,
                DiagnosticStatus::Fail => status_chars::FAIL_ASCII,
                DiagnosticStatus::Skip => status_chars::SKIP_ASCII,
            }
        }
    }

    pub fn is_tech_mode(&self) -> bool {
        self.mode == OperatingMode::Technician
    }

    pub fn effective_width(&self) -> usize {
        crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80)
    }

    pub fn calculate_widths(&self, max_label: usize, max_data: usize) -> (usize, usize) {
        let label_width = max_label.clamp(MIN_LABEL_WIDTH, MAX_LABEL_WIDTH);
        let data_width = max_data.clamp(MIN_DATA_WIDTH, MAX_DATA_WIDTH);
        (label_width, data_width)
    }

    pub fn table_width(&self, label_width: usize, data_width: usize) -> usize {
        label_width + data_width + BORDERS_PADDING
    }

    pub fn with_ascii(mut self) -> Self {
        self.use_unicode = false;
        self
    }

    pub fn with_colors(mut self, colors: bool) -> Self {
        self.use_colors = colors;
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_tech_mode(mut self) -> Self {
        self.mode = OperatingMode::Technician;
        self
    }

    pub fn with_json(mut self) -> Self {
        self.format = OutputFormat::Json;
        self
    }

    pub fn with_skip_speed(mut self) -> Self {
        self.skip_speed = true;
        self
    }

    pub fn with_speed_duration(mut self, seconds: u64) -> Self {
        self.speed_duration = seconds.max(4);
        self
    }

    pub fn with_verbose(mut self) -> Self {
        self.verbose = true;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BoxChars {
    pub top_left: char,
    pub top_right: char,
    pub bottom_left: char,
    pub bottom_right: char,
    pub horizontal: char,
    pub vertical: char,
    pub t_down: char,
    pub t_up: char,
    pub t_right: char,
    pub t_left: char,
    pub cross: char,
}

impl BoxChars {
    pub fn unicode() -> Self {
        Self {
            top_left: chars::TOP_LEFT,
            top_right: chars::TOP_RIGHT,
            bottom_left: chars::BOTTOM_LEFT,
            bottom_right: chars::BOTTOM_RIGHT,
            horizontal: chars::HORIZONTAL,
            vertical: chars::VERTICAL,
            t_down: chars::T_DOWN,
            t_up: chars::T_UP,
            t_right: chars::T_RIGHT,
            t_left: chars::T_LEFT,
            cross: chars::CROSS,
        }
    }

    pub fn ascii() -> Self {
        Self {
            top_left: ascii_chars::TOP_LEFT,
            top_right: ascii_chars::TOP_RIGHT,
            bottom_left: ascii_chars::BOTTOM_LEFT,
            bottom_right: ascii_chars::BOTTOM_RIGHT,
            horizontal: ascii_chars::HORIZONTAL,
            vertical: ascii_chars::VERTICAL,
            t_down: ascii_chars::T_DOWN,
            t_up: ascii_chars::T_UP,
            t_right: ascii_chars::T_RIGHT,
            t_left: ascii_chars::T_LEFT,
            cross: ascii_chars::CROSS,
        }
    }
}
