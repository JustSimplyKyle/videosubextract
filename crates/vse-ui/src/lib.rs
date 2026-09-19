//! Upstream-iced presentation layer for VideoSubExtract.

pub use iced;
pub type Element<'a, Message> = iced::Element<'a, Message, theme::Theme>;
pub use theme::Theme;

pub mod components;
pub mod shell;
pub mod theme;
pub mod widget;

/// Apply a closure to a value while preserving widget method-chain syntax.
pub trait Apply: Sized {
    fn apply<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Apply for T {}

pub mod font {
    /// Temporary counterpart for COSMIC's semantic semibold font token.
    pub const fn semibold() -> iced::Font {
        iced::Font::DEFAULT
    }
}
