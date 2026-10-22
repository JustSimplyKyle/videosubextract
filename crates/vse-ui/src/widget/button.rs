//! COSMIC button builders. Buttons always animate their style transitions.

use std::marker::PhantomData;

use super::animate::{Animatable, Animated, Binding};
use super::auto_widget::{AutoWidget, IntoWidget};
use crate::motion::{Mix, mix_oklab};
use iced::advanced::{Widget, renderer};
pub use iced::widget::button::*;
use iced::{Background, Element, Length, Padding, Theme};
use springs::SpringConfig;

pub fn button<'a, Message, W>(content: W) -> Button<'a, Message, W>
where
    Message: Clone + 'a,
    W: Widget<Message, Theme, iced::Renderer>,
{
    Button::new(content)
}

/// Iced's button with COSMIC styling and an animated style.
pub type Button<'a, Message, W = Element<'a, Message>, Renderer = iced::Renderer> =
    AutoWidget<Builder<'a, Message, W, Renderer>>;

pub struct Builder<'a, Message, W, Renderer> {
    inner: iced::widget::Button<'a, Message, W, Theme>,
    binding: Binding<Style>,
    config: Option<SpringConfig>,
    renderer: PhantomData<fn() -> Renderer>,
}

impl<'a, Message, W, Renderer> Builder<'a, Message, W, Renderer> {
    fn into_animated(self) -> Animated<iced::widget::Button<'a, Message, W, Theme>, Style> {
        let animated = Animated::new(self.inner, self.binding);
        match self.config {
            Some(config) => animated.with_config(config),
            None => animated,
        }
    }
}

impl<'a, Message, W, Renderer> IntoWidget for Builder<'a, Message, W, Renderer>
where
    Message: Clone + 'a,
    W: Widget<Message, Theme, Renderer>,
    Renderer: renderer::Renderer,
{
    type Message = Message;
    type Theme = Theme;
    type Renderer = Renderer;
    type Widget = Animated<iced::widget::Button<'a, Message, W, Theme>, Style>;

    fn into_widget(self) -> Self::Widget {
        self.into_animated()
    }
}

impl<'a, Message, W, Renderer> Button<'a, Message, W, Renderer>
where
    Message: Clone + 'a,
    W: Widget<Message, Theme, Renderer>,
    Renderer: renderer::Renderer,
{
    pub fn new(content: W) -> Self {
        AutoWidget::from_builder(Builder {
            inner: iced::widget::Button::new(content),
            binding: Binding::default(),
            config: None,
            renderer: PhantomData,
        })
        .style(crate::theme::button::standard)
    }

    fn map_inner(
        self,
        f: impl FnOnce(
            iced::widget::Button<'a, Message, W, Theme>,
        ) -> iced::widget::Button<'a, Message, W, Theme>,
    ) -> Self {
        self.map_builder(|mut builder| {
            builder.inner = f(builder.inner);
            builder
        })
    }

    #[must_use]
    pub fn width(self, width: impl Into<Length>) -> Self {
        self.map_inner(|inner| inner.width(width))
    }
    #[must_use]
    pub fn height(self, height: impl Into<Length>) -> Self {
        self.map_inner(|inner| inner.height(height))
    }
    #[must_use]
    pub fn padding(self, padding: impl Into<Padding>) -> Self {
        self.map_inner(|inner| inner.padding(padding))
    }
    #[must_use]
    pub fn clip(self, clip: bool) -> Self {
        self.map_inner(|inner| inner.clip(clip))
    }
    #[must_use]
    pub fn on_press(self, message: Message) -> Self {
        self.map_inner(|inner| inner.on_press(message))
    }
    #[must_use]
    pub fn on_press_maybe(self, message: Option<Message>) -> Self {
        self.map_inner(|inner| inner.on_press_maybe(message))
    }
    #[must_use]
    pub fn on_press_with(self, callback: impl Fn() -> Message + 'a) -> Self {
        self.map_inner(|inner| inner.on_press_with(callback))
    }
    #[must_use]
    pub fn on_press_maybe_with(self, callback: Option<impl Fn() -> Message + 'a>) -> Self {
        self.map_inner(|inner| inner.on_press_maybe_with(callback))
    }
    #[must_use]
    pub fn with_config(self, spring: SpringConfig) -> Self {
        self.map_builder(|mut builder| {
            builder.config = Some(spring);
            builder
        })
    }
    /// Sets the target appearance; the displayed style springs toward it.
    #[must_use]
    pub fn style(self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.map_builder(|mut builder| {
            let binding = builder.binding.clone();
            builder.inner = builder
                .inner
                .style(move |theme, status| binding.resolve(style(theme, status)));
            builder
        })
    }
    #[must_use]
    pub fn class(self, class: impl Into<StyleFn<'a, Theme>>) -> Self {
        self.style(class.into())
    }
}

impl Mix for Style {
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

impl<'a, Message, W, Renderer> Animatable for Button<'a, Message, W, Renderer>
where
    Message: Clone + 'a,
    W: Widget<Message, Theme, Renderer>,
    Renderer: renderer::Renderer,
{
    type Value = Style;
    type Widget = iced::widget::Button<'a, Message, W, Theme>;

    fn into_animated(self) -> Animated<Self::Widget, Self::Value> {
        self.into_builder().into_animated()
    }
}

impl<'a, Message: Clone + 'a, W> From<iced::widget::Button<'a, Message, W>>
    for Animated<iced::widget::Button<'a, Message, W>, iced::widget::button::Style>
{
    fn from(button: iced::widget::Button<'a, Message, W>) -> Self {
        Animated::bind(|animate| button.style(animate.wrap(crate::theme::button::standard)))
    }
}
