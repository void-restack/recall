use ratatui::style::{Color, Modifier, Style};

/// One restrained palette for the whole TUI, resolved once against `NO_COLOR`.
/// When color is off we keep the structural attributes (bold, dim, reverse) so the
/// hierarchy survives, and only drop the hues.
pub struct Theme {
    /// Focused input border and other accents.
    pub accent: Style,
    /// Matched query characters in a row.
    pub matched: Style,
    /// Descriptions, help, metadata — everything secondary.
    pub dim: Style,
    /// The first token of a command (the program).
    pub strong: Style,
    /// Destructive actions only.
    pub danger: Style,
    /// Tag chips.
    pub tag: Style,
    /// The selected row's bar.
    pub selection: Style,
}

impl Theme {
    pub fn detect() -> Self {
        // NO_COLOR disables color when present and non-empty (the informal standard).
        let colored = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
        Self::new(colored)
    }

    fn new(colored: bool) -> Self {
        let fg = |color: Color| {
            if colored {
                Style::new().fg(color)
            } else {
                Style::new()
            }
        };
        Theme {
            accent: fg(Color::Cyan),
            matched: fg(Color::Cyan).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            strong: Style::new().add_modifier(Modifier::BOLD),
            danger: fg(Color::Red),
            tag: fg(Color::Blue),
            selection: fg(Color::Cyan).add_modifier(Modifier::REVERSED),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_keeps_attributes_but_drops_hue() {
        let plain = Theme::new(false);
        assert_eq!(plain.accent.fg, None);
        assert_eq!(plain.tag.fg, None);
        // Bold/dim survive so the layout still reads.
        assert!(plain.matched.add_modifier.contains(Modifier::BOLD));
        assert!(plain.dim.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn colored_assigns_the_palette() {
        let t = Theme::new(true);
        assert_eq!(t.accent.fg, Some(Color::Cyan));
        assert_eq!(t.danger.fg, Some(Color::Red));
        assert_eq!(t.tag.fg, Some(Color::Blue));
    }
}
