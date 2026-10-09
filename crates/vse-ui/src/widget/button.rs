//! COSMIC button builders. `animate(button)` adds style transitions.

use super::animate::{Animatable, Animated, StyleBinding};
use crate::motion::{TransitionStyle, mix_oklab};
use iced::advanced::renderer;
pub use iced::widget::button::*;
use iced::{Background, Element, Length, Padding, Theme};

pub fn button<'a, Message>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    Button::new(content)
}

/// Iced's button with COSMIC styling and an optional animation hook.
pub struct Button<'a, Message, Renderer = iced::Renderer>
where
    Renderer: renderer::Renderer,
{
    inner: iced::widget::Button<'a, Message, Theme, Renderer>,
    binding: StyleBinding<Style>,
}

impl<'a, Message, Renderer: renderer::Renderer> Button<'a, Message, Renderer> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            inner: iced::widget::Button::new(content),
            binding: StyleBinding::default(),
        }
        .style(crate::theme::button::standard)
    }
    #[must_use]
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }
    #[must_use]
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }
    #[must_use]
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.inner = self.inner.padding(padding);
        self
    }
    #[must_use]
    pub fn clip(mut self, clip: bool) -> Self {
        self.inner = self.inner.clip(clip);
        self
    }
    #[must_use]
    pub fn on_press(mut self, message: Message) -> Self {
        self.inner = self.inner.on_press(message);
        self
    }
    #[must_use]
    pub fn on_press_maybe(mut self, message: Option<Message>) -> Self {
        self.inner = self.inner.on_press_maybe(message);
        self
    }
    #[must_use]
    pub fn on_press_with(mut self, callback: impl Fn() -> Message + 'a) -> Self {
        self.inner = self.inner.on_press_with(callback);
        self
    }
    #[must_use]
    pub fn on_press_maybe_with(mut self, callback: Option<impl Fn() -> Message + 'a>) -> Self {
        self.inner = self.inner.on_press_maybe_with(callback);
        self
    }
    /// Sets the appearance; wrap the button with `animate` to transition colors.
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        let binding = self.binding.clone();
        self.inner = self
            .inner
            .style(move |theme, status| binding.resolve(style(theme, status)));
        self
    }
    #[must_use]
    pub fn class(self, class: impl Into<StyleFn<'a, Theme>>) -> Self {
        self.style(class.into())
    }
}

impl TransitionStyle for Style {
    fn mix(&self, to: &Self, progress: f32) -> Self {
        let background = match (self.background, to.background) {
            (Some(Background::Color(from)), Some(Background::Color(to))) => {
                Some(mix_oklab(from, to, progress).into())
            }
            (None, Some(Background::Color(to))) => {
                Some(mix_oklab(to.scale_alpha(0.0), to, progress).into())
            }
            (Some(Background::Color(from)), None) => {
                Some(mix_oklab(from, from.scale_alpha(0.0), progress).into())
            }
            _ => to.background,
        };
        Self {
            background,
            text_color: mix_oklab(self.text_color, to.text_color, progress),
            border: iced::Border {
                color: mix_oklab(self.border.color, to.border.color, progress),
                ..to.border
            },
            shadow: iced::Shadow {
                color: mix_oklab(self.shadow.color, to.shadow.color, progress),
                ..to.shadow
            },
            ..*to
        }
    }
}

impl<'a, Message, Renderer: renderer::Renderer> Animatable for Button<'a, Message, Renderer> {
    type Style = Style;
    type Widget = iced::widget::Button<'a, Message, Theme, Renderer>;

    fn into_animated(self) -> Animated<Self::Widget, Self::Style> {
        Animated::new(self.inner, self.binding)
    }
}

impl<'a, Message: Clone + 'a, Renderer: renderer::Renderer + 'a> From<Button<'a, Message, Renderer>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(button: Button<'a, Message, Renderer>) -> Self {
        button.into_animated().into()
    }
}
