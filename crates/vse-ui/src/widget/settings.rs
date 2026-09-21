use iced::widget::{Component, column, component, container, row, text, toggler};
use iced::{Alignment, Element, Fill, Length, Renderer};

use crate::components::MessageEmitter;

#[must_use]
pub fn view_column<'a, Message: 'a>(
    items: Vec<Element<'a, Message>>,
) -> iced::widget::Column<'a, Message> {
    column(items).spacing(24).width(Fill)
}

#[must_use]
pub const fn section<'a, Message>() -> Section<'a, Message> {
    Section {
        title: None,
        items: Vec::new(),
    }
}

pub struct Section<'a, Message> {
    title: Option<String>,
    items: Vec<Element<'a, Message>>,
}

impl<'a, Message> Section<'a, Message> {
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    #[must_use]
    pub fn add(mut self, item: impl Into<Element<'a, Message>>) -> Self {
        self.items.push(item.into());
        self
    }
}

impl<'a, Message: 'a> From<Section<'a, Message>> for Element<'a, Message> {
    fn from(section: Section<'a, Message>) -> Self {
        let content = container(column(section.items).spacing(12))
            .padding(16)
            .width(Fill)
            .style(crate::theme::container::list);
        match section.title {
            Some(title) => column![text(title).size(18), content].spacing(10).into(),
            None => content.into(),
        }
    }
}

pub fn item<'a, Message: 'a>(
    label: impl Into<String>,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![text(label.into()).width(Fill), control.into()]
        .align_y(Alignment::Center)
        .spacing(16)
        .width(Length::Fill)
        .into()
}

/// a builder for a simple togglable settings item row
pub fn togglable(label: impl Into<String>) -> togglable::Builder {
    togglable::builder(label)
}

pub mod togglable {
    use super::*;

    pub(super) fn builder(label: impl Into<String>) -> Builder {
        Builder {
            label: label.into(),
            description: None,
        }
    }

    pub struct Builder {
        label: String,
        description: Option<String>,
    }

    impl<'a> Builder {
        #[must_use]
        pub fn description(mut self, description: impl Into<String>) -> Self {
            self.description = Some(description.into());
            self
        }

        pub fn toggler<Message: 'a>(
            self,
            is_enabled: bool,
            on_toggle: impl Fn(bool) -> Message + 'a,
        ) -> ToggleItem<'a, Message> {
            ToggleItem {
                label: self.label,
                description: self.description,
                is_enabled,
                on_toggle: Box::new(on_toggle),
            }
        }
    }
}

pub struct ToggleItem<'a, Message> {
    label: String,
    description: Option<String>,
    is_enabled: bool,
    on_toggle: MessageEmitter<'a, bool, Message>,
}

#[derive(Clone, Copy)]
pub enum ToggleEvent {
    Changed(bool),
}

impl<'a, Message: 'a> Component<'a, Message> for ToggleItem<'a, Message> {
    type State = ();
    type Event = ToggleEvent;

    fn update(&mut self, _: &mut Self::State, event: ToggleEvent, _: &Renderer) -> Option<Message> {
        match event {
            ToggleEvent::Changed(value) => Some((self.on_toggle)(value)),
        }
    }

    fn view(&self, _: &Self::State) -> Element<'a, Self::Event> {
        let labels = self
            .description
            .as_ref()
            .map_or_else(
                || column![text(self.label.clone())],
                |description| {
                    column![text(self.label.clone()), text(description.clone()).size(12)].spacing(2)
                },
            )
            .width(Fill);

        row![
            labels,
            toggler(self.is_enabled).on_toggle(ToggleEvent::Changed)
        ]
        .align_y(Alignment::Center)
        .spacing(16)
        .width(Fill)
        .into()
    }
}

impl<'a, Message: 'a> From<ToggleItem<'a, Message>> for Element<'a, Message> {
    fn from(item: ToggleItem<'a, Message>) -> Self {
        component(item)
    }
}
