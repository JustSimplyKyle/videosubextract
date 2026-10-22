//! Application shell built solely from upstream Iced primitives.

use iced::advanced::Widget;
use iced::widget::{Column, Stack, Widget as _, column, container, row, text};
use iced::{Alignment, Element, Fill, Length, Renderer, Theme};

use crate::widget::{AutoWidget, IntoWidget, button};

pub struct NavigationItem<Message> {
    pub label: String,
    pub selected: bool,
    pub on_press: Message,
}

pub type Shell<'a, Message> = AutoWidget<Builder<'a, Message>>;

pub struct Builder<'a, Message> {
    navigation: Vec<NavigationItem<Message>>,
    content: Element<'a, Message>,
    controls: Option<(Message, Message)>,
    toasts: Vec<(String, Message)>,
    dialog: Option<Element<'a, Message>>,
}

impl<'a, Message: Clone + 'a> Shell<'a, Message> {
    pub fn new(content: impl Widget<Message, Theme, Renderer> + 'a) -> Self {
        AutoWidget::from_builder(Builder {
            navigation: Vec::new(),
            content: content.boxed(),
            controls: None,
            toasts: Vec::new(),
            dialog: None,
        })
    }

    #[must_use]
    pub fn navigation(self, items: Vec<NavigationItem<Message>>) -> Self {
        self.map_builder(|mut shell| {
            shell.navigation = items;
            shell
        })
    }

    #[must_use]
    pub fn header_controls(self, toggle_navigation: Message, open_settings: Message) -> Self {
        self.map_builder(|mut shell| {
            shell.controls = Some((toggle_navigation, open_settings));
            shell
        })
    }

    #[must_use]
    pub fn toasts(self, toasts: Vec<(String, Message)>) -> Self {
        self.map_builder(|mut shell| {
            shell.toasts = toasts;
            shell
        })
    }

    #[must_use]
    pub fn dialog(self, dialog: impl Widget<Message, Theme, Renderer> + 'a) -> Self {
        self.map_builder(|mut shell| {
            shell.dialog = Some(dialog.boxed());
            shell
        })
    }
}

impl<'a, Message: Clone + 'a> IntoWidget for Builder<'a, Message> {
    type Message = Message;
    type Theme = Theme;
    type Renderer = Renderer;
    type Widget = Stack<Element<'a, Message>>;

    fn into_widget(self) -> Self::Widget {
        let shell = self;
        let navigation_open = !shell.navigation.is_empty();
        let nav = shell
            .navigation
            .into_iter()
            .map(|item| {
                let button = button(text(item.label)).on_press(item.on_press).width(Fill);
                if item.selected {
                    button.style(crate::theme::button::navigation_active)
                } else {
                    button.style(crate::theme::button::navigation_inactive)
                }
            })
            .collect::<Column<_>>()
            .spacing(8)
            .padding(12);

        let header = shell.controls.map(|(toggle_navigation, open_settings)| {
            row![
                button(crate::widget::icon!("open-menu-symbolic"))
                    .padding(8)
                    .style(crate::theme::button::icon)
                    .on_press(toggle_navigation),
                iced::widget::Space::new().width(Fill),
                button(crate::widget::icon!("preferences-system-symbolic"))
                    .padding(8)
                    .style(crate::theme::button::icon)
                    .on_press(open_settings),
            ]
            .align_y(Alignment::Center)
            .width(Fill)
        });
        let page = column![header, shell.content,].spacing(16);

        let content = row![
            container(nav)
                .width(if navigation_open { 200 } else { 0 })
                .height(Length::Fill)
                .style(crate::theme::container::navigation),
            container(page).padding([30, 50]).width(Fill)
        ]
        .align_y(Alignment::Start)
        .width(Length::Fill)
        .height(Length::Fill);

        let toasts = column(shell.toasts.into_iter().map(|(message, close)| {
            container(
                row![
                    text(message),
                    button(crate::widget::icon!("window-close-symbolic"))
                        .padding(4)
                        .style(crate::theme::button::icon)
                        .on_press(close),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding(12)
            .style(crate::theme::container::card)
        }))
        .spacing(8)
        .padding(20);

        let shell_layer = iced::widget::stack![content, iced::widget::bottom_center(toasts)];

        iced::widget::stack![shell_layer, shell.dialog]
            .width(Length::Fill)
            .height(Length::Fill)
    }
}
