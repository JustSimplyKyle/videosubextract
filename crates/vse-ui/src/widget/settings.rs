use iced::advanced::Widget;
use iced::widget::{Column, Component, Widget as _, column, component, row, rule, text};
use iced::{Alignment, Element, Fill, Length, Renderer, Theme};

use crate::components::MessageEmitter;
use crate::widget::switch;
use crate::widget::text::title3;
use crate::widget::{AutoWidget, IntoWidget};

#[must_use]
pub fn view_column<'a, Message: 'a>(
    items: Vec<Element<'a, Message>>,
) -> Column<Element<'a, Message>> {
    items
        .into_iter()
        .intersperse_with(|| {
            rule::horizontal(1)
                .style(crate::theme::rule::settings)
                .boxed()
        })
        .collect::<Column<_>>()
        .spacing(crate::theme::spacing().space_m)
        .width(Fill)
}

#[must_use]
pub fn section<'a, Message: 'a>() -> Section<'a, Message> {
    AutoWidget::from_builder(SectionBuilder {
        title: None,
        items: Vec::new(),
    })
}

pub type Section<'a, Message> = AutoWidget<SectionBuilder<'a, Message>>;

pub struct SectionBuilder<'a, Message> {
    title: Option<String>,
    items: Vec<Element<'a, Message>>,
}

impl<'a, Message: 'a> Section<'a, Message> {
    #[must_use]
    pub fn title(self, title: impl Into<String>) -> Self {
        self.map_builder(|mut section| {
            section.title = Some(title.into());
            section
        })
    }

    #[must_use]
    pub fn add(self, item: impl Widget<Message, Theme, Renderer> + 'a) -> Self {
        self.map_builder(|mut section| {
            section.items.push(item.boxed());
            section
        })
    }
}

impl<'a, Message: 'a> IntoWidget for SectionBuilder<'a, Message> {
    type Message = Message;
    type Theme = Theme;
    type Renderer = Renderer;
    type Widget = Column<Element<'a, Message>>;

    fn into_widget(self) -> Self::Widget {
        let spacing = crate::theme::spacing();
        let content = column(self.items).spacing(spacing.space_s).width(Fill);
        // An untitled section is just its content: the missing title is void.
        column![self.title.map(|title| text(title).size(18)), content]
            .spacing(spacing.space_s)
            .width(Fill)
    }
}

/// A settings section whose contents can be hidden behind its title row.
pub fn collapsible_section<'a, Message: Clone + 'a>(
    title: impl Into<String>,
    expanded: bool,
    on_toggle: Message,
    content: impl Widget<Message, Theme, Renderer> + 'a,
) -> Column<Element<'a, Message>> {
    let spacing = crate::theme::spacing();
    let indicator = if expanded {
        crate::widget::icon!("go-down-symbolic")
    } else {
        crate::widget::icon!("go-next-symbolic")
    };
    let header = crate::widget::button(
        row![
            title3(title.into()),
            iced::widget::Space::new().width(Fill),
            indicator,
        ]
        .align_y(Alignment::Center)
        .width(Fill),
    )
    .on_press(on_toggle)
    .padding([spacing.space_xxs, spacing.space_xs])
    .style(crate::theme::button::navigation_inactive)
    .width(Fill);

    column![
        header,
        expanded.then(|| content),
        (!expanded).then(|| rule::horizontal(1).style(crate::theme::rule::settings)),
    ]
    .spacing(spacing.space_m)
}

pub fn item<'a, Message: 'a>(
    label: impl Into<String>,
    control: impl Widget<Message, Theme, Renderer> + 'a,
) -> Item<'a, Message> {
    AutoWidget::from_builder(ItemBuilder {
        label: label.into(),
        control: control.boxed(),
        axis: ItemAxis::Horizontal,
    })
}

#[derive(Clone, Copy, Default)]
enum ItemAxis {
    #[default]
    Horizontal,
    Vertical,
}

pub type Item<'a, Message> = AutoWidget<ItemBuilder<'a, Message>>;

pub struct ItemBuilder<'a, Message> {
    label: String,
    control: Element<'a, Message>,
    axis: ItemAxis,
}

impl<'a, Message: 'a> Item<'a, Message> {
    /// Places the control below its label instead of beside it.
    #[must_use]
    pub fn vertical(self) -> Self {
        self.map_builder(|mut item| {
            item.axis = ItemAxis::Vertical;
            item
        })
    }
}

impl<'a, Message: 'a> IntoWidget for ItemBuilder<'a, Message> {
    type Message = Message;
    type Theme = Theme;
    type Renderer = Renderer;
    type Widget = Element<'a, Message>;

    fn into_widget(self) -> Self::Widget {
        let label = text(self.label)
            .size(14)
            .style(crate::theme::text::settings_label);

        match self.axis {
            ItemAxis::Horizontal => row![label.width(Fill), self.control]
                .align_y(Alignment::Center)
                .spacing(crate::theme::spacing().space_m)
                .width(Fill)
                .boxed(),
            ItemAxis::Vertical => column![label, self.control]
                .spacing(crate::theme::spacing().space_xs)
                .width(Length::Fill)
                .boxed(),
        }
    }
}

/// a builder for a simple togglable settings item row
pub fn togglable(label: impl Into<String>) -> togglable::Builder {
    togglable::builder(label)
}

pub mod togglable {
    use std::rc::Rc;

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
        ) -> impl Widget<Message, Theme, Renderer> + 'a {
            component(ToggleItem {
                label: self.label,
                description: self.description,
                is_enabled,
                on_toggle: Rc::new(on_toggle),
            })
        }
    }
}

struct ToggleItem<'a, Message> {
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

    fn view(&self, _: &Self::State) -> impl Widget<Self::Event, Theme, Renderer> + 'a {
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
    }
}
