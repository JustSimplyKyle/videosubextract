//! Presentation components that have no direct upstream-Iced sibling.

use iced::widget::{column, container, row, text};
use iced::{Element, Fill, Length};

/// Maps state emitted by a component into its parent's message type.
pub type MessageEmitter<'a, State, Message> = Box<dyn Fn(State) -> Message + 'a>;

/// A COSMIC-style modal surface. [`crate::shell::Shell`] owns overlay placement.
pub fn dialog<'a, Message: 'a>(
    title: impl Into<String>,
    body: impl Into<Element<'a, Message>>,
    close: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let actions = row![iced::widget::Space::new().width(Fill), close.into()];
    container(column![text(title.into()).size(20), body.into(), actions].spacing(24))
        .padding(24)
        .width(570)
        .height(Length::Shrink)
        .style(crate::theme::Container::Dialog::style)
        .into()
}
