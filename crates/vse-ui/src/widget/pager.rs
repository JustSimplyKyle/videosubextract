//! Horizontally paged content with a dot indicator, driven by a
//! [`SingleSelectModel`]. The active entry is the visible page.
//!
//! The dots switch pages, and scrolling or swiping over the indicator steps
//! to the neighbouring page (see [`swipe_helper`](super::swipe_helper)).
//!
//! ```ignore
//! widget::pager::horizontal(&self.pages).on_activate(Message::SelectPage)
//! ```

use std::rc::Rc;
use std::time::Instant;

use iced::widget::{row, space};
use iced::{Alignment, Color, Element};

use super::segmented_button::{Entity, SingleSelectModel};
use crate::motion::{Motion, presets};
use crate::theme::{COSMIC, color};
use crate::widget::{self, button};
use crate::{Apply, components::MessageEmitter};

#[must_use]
pub const fn horizontal<T>(model: &SingleSelectModel<T>) -> PagerBuilder<'_, T> {
    PagerBuilder { model }
}

pub struct PagerBuilder<'a, T> {
    model: &'a SingleSelectModel<T>,
}

pub struct Pager<'a, T, Message> {
    model: &'a SingleSelectModel<T>,
    on_activate: MessageEmitter<'a, Entity, Message>,
}

impl<'a, T> PagerBuilder<'a, T> {
    pub fn on_activate<Message>(self, f: impl Fn(Entity) -> Message + 'a) -> Pager<'a, T, Message> {
        Pager {
            model: self.model,
            on_activate: Rc::new(f),
        }
    }
}

impl<'a, T, Message: 'a + Clone> From<Pager<'a, T, Message>> for Element<'a, Message> {
    fn from(pager: Pager<'a, T, Message>) -> Self {
        let model = pager.model;
        widget::swipe_helper(indicator(model))
            .on_next_maybe(model.next_entity())
            .on_previous_maybe(model.previous_entity())
            .apply(Element::from)
            .map(move |id| (pager.on_activate)(id))
    }
}

fn indicator<T>(model: &SingleSelectModel<T>) -> Element<'_, Entity> {
    let entries = model.entries();
    let active = model.active();

    let dots = entries
        .keys()
        .map(|id| dot(*id == active, *id))
        .apply(row)
        .spacing(4)
        .align_y(Alignment::Center);
    let label = iced::widget::text(entries[&active].text()).size(12);
    row![dots, label]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

/// The active page is a wider accent pill; the rest are small neutral dots.
fn dot<'a>(active: bool, on_press: Entity) -> Element<'a, Entity> {
    let (width, tint) = if active {
        (14., color(COSMIC.accent.base))
    } else {
        (6., color(COSMIC.background(false).on).scale_alpha(0.35))
    };

    iced::widget::transition(
        (width, presets::gentle()),
        move || Motion::new(width, Instant::now()),
        move |motion: &Motion, _| space().width(motion.value()).height(6),
    )
    .apply(button)
    .padding(0)
    .on_press(on_press)
    .style(move |_, _| button::Style {
        background: Some(tint.into()),
        border: iced::border::rounded(3),
        text_color: Color::TRANSPARENT,
        ..button::Style::default()
    })
    .into()
}
