//! Upstream-iced presentation layer for VideoSubExtract.
#![feature(iter_intersperse, default_field_values, impl_trait_in_assoc_type)]

extern crate self as vse_ui;

pub use iced;
pub type Element<'a, Message> = iced::Element<'a, Message, theme::Theme>;
pub use theme::Theme;

pub mod components;
pub mod motion;
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

pub trait ApplyConditional: Sized {
    #[must_use]
    fn apply_if<T>(self, flag: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if flag { f(self) } else { self }
    }

    #[must_use]
    fn apply_if_some<T>(self, value: Option<T>, f: impl FnOnce(Self, T) -> Self) -> Self {
        match value {
            Some(value) => f(self, value),
            None => self,
        }
    }

    #[must_use]
    fn apply_if_ok<T, E>(self, result: Result<T, E>, f: impl FnOnce(Self, T) -> Self) -> Self {
        match result {
            Ok(value) => f(self, value),
            Err(_) => self,
        }
    }

    #[must_use]
    fn apply_if_ok_ref<'a, T, E>(
        self,
        result: &'a Result<T, E>,
        f: impl FnOnce(Self, &'a T) -> Self,
    ) -> Self {
        match result {
            Ok(value) => f(self, value),
            Err(_) => self,
        }
    }

    #[must_use]
    fn apply_if_some_ref<'a, T>(
        self,
        option: &'a Option<T>,
        f: impl FnOnce(Self, &'a T) -> Self,
    ) -> Self {
        match option {
            Some(value) => f(self, value),
            None => self,
        }
    }
}

impl<T> ApplyConditional for T {}

pub mod font {
    /// Temporary counterpart for COSMIC's semantic semibold font token.
    pub const fn semibold() -> iced::Font {
        iced::Font::DEFAULT
    }
}
