use iced::widget::{Component, column, component, row, rule, text, toggler};
use iced::{Alignment, Element, Fill, Length, Renderer};

use crate::components::MessageEmitter;
use crate::widget::switch;
use crate::widget::text::title3;

#[must_use]
pub fn view_column<'a, Message: 'a>(
    items: Vec<Element<'a, Message>>,
) -> iced::widget::Column<'a, Message> {
    items
        .into_iter()
        .intersperse_with(|| {
            rule::horizontal(1)
                .style(crate::theme::rule::settings)
                .into()
        })
        .collect::<iced::widget::Column<_>>()
        .spacing(crate::theme::spacing().space_m)
        .width(Fill)
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
        let spacing = crate::theme::spacing();
        let content = column(section.items).spacing(spacing.space_s).width(Fill);
        match section.title {
            Some(title) => column![text(title).size(18), content]
                .spacing(spacing.space_s)
                .width(Fill)
                .into(),
            None => content.into(),
        }
    }
}

/// A settings section whose contents can be hidden behind its title row.
pub fn collapsible_section<'a, Message: Clone + 'a>(
    title: impl Into<String>,
    expanded: bool,
    on_toggle: Message,
    content: impl Into<Element<'a, Message>>,
) -> ::iced::widget::Column<'a, Message> {
    let spacing = crate::theme::spacing();
    let indicator = if expanded {
        "go-down-symbolic"
    } else {
        "go-next-symbolic"
    };
    let header = crate::widget::button(
        row![
            title3(title.into()),
            iced::widget::Space::new().width(Fill),
            crate::widget::icon::from_name(indicator),
        ]
        .align_y(Alignment::Center)
        .width(Fill),
    )
    .on_press(on_toggle)
    .padding([spacing.space_xxs, spacing.space_xs])
    .style(crate::theme::button::navigation_inactive)
    .width(Fill);

    ::iced::widget::Column::with_capacity(2)
        .push(header)
        .push(if expanded {
            content.into()
        } else {
            rule::horizontal(1)
                .style(crate::theme::rule::settings)
                .into()
        })
        .spacing(spacing.space_m)
}

pub fn item<'a, Message: 'a>(
    label: impl Into<String>,
    control: impl Into<Element<'a, Message>>,
) -> Item<'a, Message> {
    Item {
        label: label.into(),
        control: control.into(),
        axis: ItemAxis::Horizontal,
    }
}

#[derive(Clone, Copy, Default)]
enum ItemAxis {
    #[default]
    Horizontal,
    Vertical,
}

pub struct Item<'a, Message> {
    label: String,
    control: Element<'a, Message>,
    axis: ItemAxis,
}

impl<Message> Item<'_, Message> {
    /// Places the control below its label instead of beside it.
    #[must_use]
    pub const fn vertical(mut self) -> Self {
        self.axis = ItemAxis::Vertical;
        self
    }
}

impl<'a, Message: 'a> From<Item<'a, Message>> for Element<'a, Message> {
    fn from(item: Item<'a, Message>) -> Self {
        let label = text(item.label)
            .size(14)
            .style(crate::theme::text::settings_label);

        match item.axis {
            ItemAxis::Horizontal => row![label.width(Fill), item.control]
                .align_y(Alignment::Center)
                .spacing(crate::theme::spacing().space_m)
                .width(Fill)
                .into(),
            ItemAxis::Vertical => column![label, item.control]
                .spacing(crate::theme::spacing().space_xs)
                .width(Length::Fill)
                .into(),
        }
    }
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

    fn update(&self, _: &mut Self::State, event: ToggleEvent, _: &Renderer) -> Option<Message> {
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
                    column![
                        text(self.label.clone()),
                        text(description.clone())
                            .size(12)
                            .style(crate::theme::text::settings_label)
                    ]
                    .spacing(crate::theme::spacing().space_xxs)
                },
            )
            .width(Fill);

        row![
            labels,
            switch::switch(self.is_enabled)
                .on_toggle(ToggleEvent::Changed)
                .with_size(switch::Size::small())
        ]
        .align_y(Alignment::Center)
        .spacing(crate::theme::spacing().space_m)
        .width(Fill)
        .into()
    }
}

impl<'a, Message: 'a> From<ToggleItem<'a, Message>> for Element<'a, Message> {
    fn from(item: ToggleItem<'a, Message>) -> Self {
        component(item)
    }
}
