//! Application shell built solely from upstream Iced primitives.

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};

pub struct NavigationItem<Message> {
    pub label: String,
    pub selected: bool,
    pub on_press: Message,
}

pub struct Shell<'a, Message> {
    title: String,
    description: String,
    navigation: Vec<NavigationItem<Message>>,
    content: Element<'a, Message>,
    controls: Option<(Message, Message)>,
    toasts: Vec<(String, Message)>,
    dialog: Option<Element<'a, Message>>,
}

impl<'a, Message: 'a> Shell<'a, Message> {
    pub fn new(
        title: impl Into<String>,
        description: impl Into<String>,
        content: impl Into<Element<'a, Message>>,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            navigation: Vec::new(),
            content: content.into(),
            controls: None,
            toasts: Vec::new(),
            dialog: None,
        }
    }

    #[must_use]
    pub fn navigation(mut self, items: Vec<NavigationItem<Message>>) -> Self {
        self.navigation = items;
        self
    }

    #[must_use]
    pub fn header_controls(mut self, toggle_navigation: Message, open_settings: Message) -> Self {
        self.controls = Some((toggle_navigation, open_settings));
        self
    }

    #[must_use]
    pub fn toasts(mut self, toasts: Vec<(String, Message)>) -> Self {
        self.toasts = toasts;
        self
    }

    #[must_use]
    pub fn dialog(mut self, dialog: Option<impl Into<Element<'a, Message>>>) -> Self {
        self.dialog = dialog.map(Into::into);
        self
    }
}

impl<'a, Message: Clone + 'a> From<Shell<'a, Message>> for Element<'a, Message> {
    fn from(shell: Shell<'a, Message>) -> Self {
        let navigation_open = !shell.navigation.is_empty();
        let nav =
            shell
                .navigation
                .into_iter()
                .fold(column![].spacing(8).padding(12), |nav, item| {
                    let button = button(text(item.label)).on_press(item.on_press).width(Fill);
                    nav.push(if item.selected {
                        button.style(crate::theme::button::navigation_active)
                    } else {
                        button.style(crate::theme::button::navigation_inactive)
                    })
                });

        let header = shell.controls.map(|(toggle_navigation, open_settings)| {
            row![
                button(crate::widget::icon::from_name("open-menu-symbolic"))
                    .padding(8)
                    .style(crate::theme::button::icon)
                    .on_press(toggle_navigation),
                iced::widget::Space::new().width(Fill),
                button(crate::widget::icon::from_name(
                    "preferences-system-symbolic"
                ))
                .padding(8)
                .style(crate::theme::button::icon)
                .on_press(open_settings),
            ]
            .align_y(Alignment::Center)
            .width(Fill)
        });
        let page = column![
            header,
            text(shell.title).size(36),
            text(shell.description).size(14),
            shell.content,
        ]
        .spacing(16);

        let content: Element<'a, Message> = row![
            container(nav)
                .width(if navigation_open { 200 } else { 0 })
                .height(Length::Fill)
                .style(crate::theme::container::navigation),
            container(page).padding([30, 50]).width(Fill)
        ]
        .align_y(Alignment::Start)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        let toasts = column(shell.toasts.into_iter().map(|(message, close)| {
            container(
                row![
                    text(message),
                    button(crate::widget::icon::from_name("window-close-symbolic"))
                        .padding(4)
                        .style(crate::theme::button::icon)
                        .on_press(close),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding(12)
            .style(crate::theme::container::card)
            .into()
        }))
        .spacing(8)
        .padding(20);

        let dialog = shell.dialog;
        let shell: Element<'a, Message> =
            iced::widget::stack![content, iced::widget::bottom_right(toasts)].into();

        if let Some(dialog) = dialog {
            iced::widget::stack![
                shell,
                container(dialog)
                    .center(Length::Fill)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(|_| container::background(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.55))),
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            shell
        }
    }
}
