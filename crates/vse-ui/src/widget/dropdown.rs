//! Apply COSMIC color transitions to a fully configured native pick list.

use super::animate::Animated;
use crate::motion::{TransitionStyle, mix_oklab};
use iced::widget::pick_list::{PickList, Style};
use iced::{Background, Theme};
use std::borrow::Borrow;

pub type AnimatedDropdown<'a, T, L, V, Message> =
    Animated<PickList<'a, T, L, V, Message, Theme>, Style>;

impl<'a, T, L, V, Message> From<PickList<'a, T, L, V, Message, Theme>>
    for AnimatedDropdown<'a, T, L, V, Message>
where
    T: PartialEq + Clone,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
    Message: Clone,
{
    fn from(dropdown: PickList<'a, T, L, V, Message, Theme>) -> Self {
        Animated::bind(|binding| {
            dropdown.style(move |theme, status| {
                binding.resolve(crate::theme::pick_list::standard(theme, status))
            })
        })
    }
}

impl TransitionStyle for Style {
    fn mix(&self, to: &Self, progress: f32) -> Self {
        let background = match (self.background, to.background) {
            (Background::Color(from), Background::Color(to)) => {
                mix_oklab(from, to, progress).into()
            }
            _ => to.background,
        };
        Self {
            background,
            text_color: mix_oklab(self.text_color, to.text_color, progress),
            placeholder_color: mix_oklab(self.placeholder_color, to.placeholder_color, progress),
            handle_color: mix_oklab(self.handle_color, to.handle_color, progress),
            border: iced::Border {
                color: mix_oklab(self.border.color, to.border.color, progress),
                ..to.border
            },
        }
    }
}
