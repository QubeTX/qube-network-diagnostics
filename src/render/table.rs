use crate::config::BoxChars;

pub struct TableRenderer {
    label_width: usize,
    data_width: usize,
    chars: BoxChars,
    total_width: usize,
}

impl TableRenderer {
    pub fn new(label_width: usize, data_width: usize, chars: BoxChars) -> Self {
        let total_width = label_width + data_width + 7;
        Self {
            label_width,
            data_width,
            chars,
            total_width,
        }
    }

    pub fn render_top_header(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.top_left);
        for _ in 0..(self.total_width - 2) {
            line.push(self.chars.t_down);
        }
        line.push(self.chars.top_right);
        line.push('\n');
        line
    }

    pub fn render_header_bottom(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.t_right);
        for _ in 0..(self.total_width - 2) {
            line.push(self.chars.t_up);
        }
        line.push(self.chars.t_left);
        line.push('\n');
        line
    }

    pub fn render_centered(&self, text: &str) -> String {
        let text_len = visible_len(text);
        let inner_width = self.total_width - 2;

        let padding = if text_len >= inner_width {
            0
        } else {
            (inner_width - text_len) / 2
        };
        let extra = if text_len >= inner_width {
            0
        } else {
            (inner_width - text_len) % 2
        };

        let mut line = String::new();
        line.push(self.chars.vertical);
        line.push_str(&" ".repeat(padding));

        if text_len > inner_width {
            let truncated = truncate_visible(text, inner_width.saturating_sub(3));
            line.push_str(&truncated);
            line.push_str("...");
        } else {
            line.push_str(text);
        }

        line.push_str(&" ".repeat(padding + extra));
        line.push(self.chars.vertical);
        line.push('\n');
        line
    }

    pub fn render_full_top(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.top_left);
        for _ in 0..(self.total_width - 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.top_right);
        line.push('\n');
        line
    }

    pub fn render_top_divider(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.t_right);
        for _ in 0..(self.label_width + 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.t_down);
        for _ in 0..(self.data_width + 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.t_left);
        line.push('\n');
        line
    }

    pub fn render_middle_divider(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.t_right);
        for _ in 0..(self.label_width + 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.cross);
        for _ in 0..(self.data_width + 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.t_left);
        line.push('\n');
        line
    }

    pub fn render_bottom_divider(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.t_right);
        for _ in 0..(self.label_width + 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.t_up);
        for _ in 0..(self.data_width + 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.t_left);
        line.push('\n');
        line
    }

    pub fn render_footer(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.bottom_left);
        for _ in 0..(self.label_width + 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.t_up);
        for _ in 0..(self.data_width + 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.bottom_right);
        line.push('\n');
        line
    }

    pub fn render_full_divider(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.t_right);
        for _ in 0..(self.total_width - 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.t_left);
        line.push('\n');
        line
    }

    pub fn render_full_bottom(&self) -> String {
        let mut line = String::new();
        line.push(self.chars.bottom_left);
        for _ in 0..(self.total_width - 2) {
            line.push(self.chars.horizontal);
        }
        line.push(self.chars.bottom_right);
        line.push('\n');
        line
    }

    pub fn render_row(&self, label: &str, value: &str) -> String {
        let label_display = fit_string(label, self.label_width);
        let value_display = fit_string(value, self.data_width);

        let mut line = String::new();
        line.push(self.chars.vertical);
        line.push(' ');
        line.push_str(&label_display);
        line.push(' ');
        line.push(self.chars.vertical);
        line.push(' ');
        line.push_str(&value_display);
        line.push(' ');
        line.push(self.chars.vertical);
        line.push('\n');
        line
    }

    pub fn render_span_row(&self, text: &str) -> String {
        let inner = self.total_width - 2;
        let display = fit_string(text, inner);

        let mut line = String::new();
        line.push(self.chars.vertical);
        line.push_str(&display);
        line.push(self.chars.vertical);
        line.push('\n');
        line
    }

    pub fn total_width(&self) -> usize {
        self.total_width
    }

    pub fn label_width(&self) -> usize {
        self.label_width
    }

    pub fn data_width(&self) -> usize {
        self.data_width
    }
}

/// Count visible characters (excluding ANSI escape sequences)
pub fn visible_len(s: &str) -> usize {
    let mut count = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            count += 1;
        }
    }
    count
}

pub fn fit_string(s: &str, width: usize) -> String {
    let vis_len = visible_len(s);
    if vis_len > width {
        // Truncate by visible characters, preserving escape sequences
        if width <= 3 {
            truncate_visible(s, width)
        } else {
            let truncated = truncate_visible(s, width - 3);
            // Append reset + ellipsis if string had colors
            if s.contains('\x1b') {
                format!("{}\x1b[0m...", truncated)
            } else {
                format!("{}...", truncated)
            }
        }
    } else {
        // Pad with spaces based on visible length
        let padding = width - vis_len;
        format!("{}{}", s, " ".repeat(padding))
    }
}

/// Truncate a string to `max_visible` visible characters, preserving ANSI escapes
fn truncate_visible(s: &str, max_visible: usize) -> String {
    let mut result = String::new();
    let mut visible_count = 0;
    let mut in_escape = false;

    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
            result.push(c);
        } else if in_escape {
            result.push(c);
            if c == 'm' {
                in_escape = false;
            }
        } else {
            if visible_count >= max_visible {
                break;
            }
            result.push(c);
            visible_count += 1;
        }
    }

    result
}

pub struct ReportBuilder {
    renderer: TableRenderer,
    output: String,
}

impl ReportBuilder {
    pub fn new(label_width: usize, data_width: usize, chars: BoxChars) -> Self {
        Self {
            renderer: TableRenderer::new(label_width, data_width, chars),
            output: String::new(),
        }
    }

    pub fn header(mut self, title: &str, subtitle: &str) -> Self {
        self.output.push_str(&self.renderer.render_top_header());
        self.output.push_str(&self.renderer.render_header_bottom());
        self.output.push_str(&self.renderer.render_centered(title));
        self.output
            .push_str(&self.renderer.render_centered(subtitle));
        self.output.push_str(&self.renderer.render_top_divider());
        self
    }

    pub fn section_header(mut self, text: &str) -> Self {
        self.output.push_str(&self.renderer.render_bottom_divider());
        self.output
            .push_str(&self.renderer.render_span_row(&format!("  {}", text)));
        self.output.push_str(&self.renderer.render_top_divider());
        self
    }

    pub fn row(mut self, label: &str, value: &str) -> Self {
        self.output
            .push_str(&self.renderer.render_row(label, value));
        self
    }

    pub fn divider(mut self) -> Self {
        self.output.push_str(&self.renderer.render_middle_divider());
        self
    }

    pub fn full_top_border(mut self) -> Self {
        self.output.push_str(&self.renderer.render_full_top());
        self
    }

    pub fn span_row(mut self, text: &str) -> Self {
        self.output.push_str(&self.renderer.render_span_row(text));
        self
    }

    pub fn finish(mut self) -> String {
        self.output.push_str(&self.renderer.render_bottom_divider());
        self.output.push_str(&self.renderer.render_full_bottom());
        self.output
    }

    pub fn build(self) -> String {
        self.output
    }

    pub fn renderer(&self) -> &TableRenderer {
        &self.renderer
    }
}
