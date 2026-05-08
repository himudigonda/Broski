//! Color palette. Kept tiny and explicit so a future theme system can swap it.

use ratatui::style::Color;

pub const FG: Color = Color::White;
pub const ACCENT: Color = Color::Cyan;
pub const RUNNING: Color = Color::Yellow;
pub const DONE: Color = Color::Green;
pub const CACHED: Color = Color::Blue;
pub const FAILED: Color = Color::Red;
pub const QUEUED: Color = Color::DarkGray;
pub const STDERR: Color = Color::LightRed;
pub const STDOUT: Color = Color::Gray;
pub const HELP: Color = Color::DarkGray;
