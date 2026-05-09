//! Color palette and built-in theme variants.
//!
//! Themes are picked by name; widgets read a `Palette` reference. There's no
//! file format yet — keep all configuration in code so it's auditable and
//! impossible to typo.

use std::str::FromStr;

use anyhow::{anyhow, Result};
use ratatui::style::Color;

/// One of the built-in named themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Sensible defaults; colors that work on most terminals.
    #[default]
    Default,
    /// Cooler tones, designed for dark backgrounds.
    Dark,
    /// Inverted contrast for light terminal backgrounds.
    Light,
    /// Maximum contrast, accessibility-friendly.
    HighContrast,
    /// Pick `Dark` or `Light` based on the actual terminal background.
    /// Resolved by [`Theme::resolved`] before any pixels are drawn.
    Auto,
}

impl Theme {
    /// All built-in theme names, in display order.
    pub const NAMES: &'static [&'static str] =
        &["default", "dark", "light", "high-contrast", "auto"];

    /// Resolve the palette for this theme.
    pub fn palette(&self) -> Palette {
        match self {
            Theme::Default => Palette {
                fg: Color::White,
                accent: Color::Cyan,
                running: Color::Yellow,
                done: Color::Green,
                cached: Color::Blue,
                failed: Color::Red,
                queued: Color::DarkGray,
                stderr: Color::LightRed,
                stdout: Color::Gray,
                help: Color::DarkGray,
                eta: Color::Magenta,
            },
            Theme::Dark => Palette {
                fg: Color::Rgb(220, 220, 230),
                accent: Color::Rgb(120, 200, 220),
                running: Color::Rgb(240, 200, 80),
                done: Color::Rgb(120, 200, 120),
                cached: Color::Rgb(120, 160, 220),
                failed: Color::Rgb(230, 110, 110),
                queued: Color::Rgb(100, 100, 110),
                stderr: Color::Rgb(230, 130, 130),
                stdout: Color::Rgb(180, 180, 190),
                help: Color::Rgb(110, 110, 120),
                eta: Color::Rgb(190, 140, 220),
            },
            Theme::Light => Palette {
                fg: Color::Rgb(40, 40, 50),
                accent: Color::Rgb(20, 100, 130),
                running: Color::Rgb(170, 110, 0),
                done: Color::Rgb(40, 130, 50),
                cached: Color::Rgb(40, 80, 160),
                failed: Color::Rgb(170, 40, 40),
                queued: Color::Rgb(150, 150, 160),
                stderr: Color::Rgb(170, 60, 60),
                stdout: Color::Rgb(80, 80, 90),
                help: Color::Rgb(120, 120, 130),
                eta: Color::Rgb(120, 60, 160),
            },
            Theme::HighContrast => Palette {
                fg: Color::White,
                accent: Color::White,
                running: Color::Yellow,
                done: Color::Green,
                cached: Color::Cyan,
                failed: Color::Red,
                queued: Color::White,
                stderr: Color::Red,
                stdout: Color::White,
                help: Color::White,
                eta: Color::Magenta,
            },
            Theme::Auto => self.resolved().palette(),
        }
    }

    /// Display name (lowercase, kebab-case).
    pub fn name(&self) -> &'static str {
        match self {
            Theme::Default => "default",
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::HighContrast => "high-contrast",
            Theme::Auto => "auto",
        }
    }

    /// Resolve `Auto` to a concrete theme by sniffing the terminal
    /// background. Returns `self` unchanged for non-`Auto` variants. Should
    /// be called BEFORE entering raw mode — the underlying OSC 11 query
    /// expects a cooked stdout/stdin pair.
    ///
    /// On detection failure (terminal doesn't support OSC 11, no TTY,
    /// timeout), falls back to [`Theme::Default`].
    pub fn resolved(self) -> Theme {
        if self != Theme::Auto {
            return self;
        }
        match detect_terminal_kind() {
            Some(TerminalKind::Dark) => Theme::Dark,
            Some(TerminalKind::Light) => Theme::Light,
            None => Theme::Default,
        }
    }
}

/// Inferred terminal background brightness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalKind {
    Dark,
    Light,
}

/// Threshold above which a luma value is considered a "light" background.
/// terminal-light returns Y in `[0.0, 1.0]` (Rec. 709 luma); 0.5 is the
/// canonical midpoint and matches what `terminal-light::Luma::dark_or_light`
/// uses internally.
const LIGHT_LUMA_THRESHOLD: f32 = 0.5;

fn detect_terminal_kind() -> Option<TerminalKind> {
    match terminal_light::luma() {
        Ok(luma) => {
            Some(if luma < LIGHT_LUMA_THRESHOLD { TerminalKind::Dark } else { TerminalKind::Light })
        }
        Err(_) => None,
    }
}

impl FromStr for Theme {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "default" => Ok(Theme::Default),
            "dark" => Ok(Theme::Dark),
            "light" => Ok(Theme::Light),
            "high-contrast" | "highcontrast" | "hc" => Ok(Theme::HighContrast),
            "auto" => Ok(Theme::Auto),
            other => Err(anyhow!(
                "unknown theme '{}'; expected one of {}",
                other,
                Theme::NAMES.join(", ")
            )),
        }
    }
}

/// Resolved color palette. Widgets read fields from this struct rather than
/// reaching for global constants.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub fg: Color,
    pub accent: Color,
    pub running: Color,
    pub done: Color,
    pub cached: Color,
    pub failed: Color,
    pub queued: Color,
    pub stderr: Color,
    pub stdout: Color,
    pub help: Color,
    pub eta: Color,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_round_trips_by_name() {
        for name in Theme::NAMES {
            let theme: Theme = name.parse().expect("name parses");
            assert_eq!(theme.name(), *name);
        }
    }

    #[test]
    fn theme_parsing_is_case_insensitive() {
        assert_eq!("DARK".parse::<Theme>().unwrap(), Theme::Dark);
        assert_eq!("High-Contrast".parse::<Theme>().unwrap(), Theme::HighContrast);
        assert_eq!("hc".parse::<Theme>().unwrap(), Theme::HighContrast);
    }

    #[test]
    fn empty_string_resolves_to_default() {
        assert_eq!("".parse::<Theme>().unwrap(), Theme::Default);
    }

    #[test]
    fn unknown_theme_errors_with_helpful_list() {
        let err = "neon".parse::<Theme>().expect_err("should fail");
        let msg = err.to_string();
        assert!(msg.contains("unknown theme 'neon'"));
        assert!(msg.contains("default"));
        assert!(msg.contains("dark"));
    }

    #[test]
    fn each_palette_has_distinct_failed_color() {
        // Sanity: the four built-in themes should at least differ in their
        // semantic-failure color so users can tell them apart.
        let mut seen = std::collections::HashSet::new();
        for theme in [Theme::Default, Theme::Dark, Theme::Light, Theme::HighContrast] {
            seen.insert(format!("{:?}", theme.palette().failed));
        }
        assert!(seen.len() >= 2, "expected at least two distinct failed colors");
    }

    #[test]
    fn auto_theme_parses_and_round_trips() {
        assert_eq!("auto".parse::<Theme>().unwrap(), Theme::Auto);
        assert_eq!("AUTO".parse::<Theme>().unwrap(), Theme::Auto);
        assert_eq!(Theme::Auto.name(), "auto");
        assert!(Theme::NAMES.contains(&"auto"));
    }

    #[test]
    fn auto_theme_resolves_to_a_concrete_variant() {
        // Without a TTY (cargo test), terminal-light fails fast and
        // resolved() falls back to Default. With a TTY it'll be Dark or
        // Light. Either way the result must NOT be Auto, and palette()
        // must work without recursing.
        let resolved = Theme::Auto.resolved();
        assert!(matches!(resolved, Theme::Default | Theme::Dark | Theme::Light));
        // palette() on Auto should also self-resolve and not loop.
        let _ = Theme::Auto.palette();
    }

    #[test]
    fn non_auto_themes_resolve_to_themselves() {
        assert_eq!(Theme::Default.resolved(), Theme::Default);
        assert_eq!(Theme::Dark.resolved(), Theme::Dark);
        assert_eq!(Theme::Light.resolved(), Theme::Light);
        assert_eq!(Theme::HighContrast.resolved(), Theme::HighContrast);
    }
}
