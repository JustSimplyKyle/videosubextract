//! Presentation components that have no direct upstream-Iced sibling.

use std::rc::Rc;

use iced::widget::{Widget, column, container, row, text};
use iced::{Fill, Length};

/// Maps state emitted by a component into its parent's message type.
pub type MessageEmitter<'a, State, Message> = Rc<dyn Fn(State) -> Message + 'a>;

/// A COSMIC-style dialog panel. [`crate::widget::dialog`] owns modal presentation.
pub fn dialog<'a, Message: 'a>(
    title: impl Into<String>,
    body: impl Widget<Message> + 'a,
    close: impl Widget<Message> + 'a,
) -> impl Widget<Message> + 'a {
    let actions = row![iced::widget::Space::new().width(Fill), close];
    container(column![text(title.into()).size(20), body, actions].spacing(24))
        .padding(24)
        .width(570)
        .height(Length::Shrink)
        .style(crate::theme::container::dialog)
}
