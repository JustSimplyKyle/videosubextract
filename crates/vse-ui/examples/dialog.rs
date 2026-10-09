//! Run with `cargo run -p vse-ui --example dialog`.
use iced::widget::{column, container, text};
use iced::{Element, Fill, Task};
use vse_ui::{components, shell::Shell, widget};

fn main() -> iced::Result {
    iced::application(Demo::default, Demo::update, Demo::view)
        .title("Animated dialog")
        .theme(|_: &Demo| vse_ui::theme::iced_theme())
        .window_size((800, 600))
        .run()
}

#[derive(Default)]
struct Demo {
    open: bool,
    value: String,
}

#[derive(Debug, Clone)]
enum Message {
    Open,
    Close,
    Edit(String),
}

impl Demo {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Open => self.open = true,
            Message::Close => self.open = false,
            Message::Edit(value) => self.value = value,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let page = container(
            column![
                text("Component-owned dialog animation").size(24),
                widget::button("Open dialog").on_press(Message::Open),
            ]
            .spacing(vse_ui::theme::spacing().space_m),
        )
        .center(Fill);

        Shell::new(page)
            .dialog(widget::dialog(self.open, || {
                components::dialog(
                    "Animated dialog",
                    column![
                        text("The component retains this panel until closing settles."),
                        widget::text_input("Type here", &self.value).on_input(Message::Edit),
                    ]
                    .spacing(vse_ui::theme::spacing().space_s),
                    widget::button("Close").on_press(Message::Close),
                )
            }))
            .into()
    }
}
