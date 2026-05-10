use crate::config::Config;

pub fn render_bar(value: f64, max: f64, width: usize, config: &Config) -> String {
    let (filled_char, empty_char) = config.bar_chars();
    let ratio = (value / max).clamp(0.0, 1.0);
    let filled = (ratio * width as f64) as usize;
    let empty = width - filled;

    format!(
        "{}{}",
        std::iter::repeat_n(filled_char, filled).collect::<String>(),
        std::iter::repeat_n(empty_char, empty).collect::<String>()
    )
}

pub fn render_percentage_bar(percentage: f64, width: usize, config: &Config) -> String {
    render_bar(percentage, 100.0, width, config)
}
