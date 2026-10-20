//! Run with `cargo run -p vse-ui --example pager`.
use iced::widget::{self, column, container, row, text};
use iced::{Element, Fill, Task};
use vse_ui::Apply;
use vse_ui::widget::segmented_button::{Entity, SingleSelectModel, SingleSelectModelBuilder};
use vse_ui::widget::{button, pager};

fn main() -> iced::Result {
    iced::application(Demo::new, Demo::update, Demo::view)
        .title("Pager")
        .theme(|_: &Demo| vse_ui::theme::iced_theme())
        .window_size((320, 240))
        .run()
}

struct Demo<'a> {
    pages: SingleSelectModel<Box<dyn Fn() -> Element<'a, Message>>>,
}

#[derive(Debug, Clone)]
enum Message {
    SelectPage(Entity),
    Previous,
    Next,
}

impl<'a> Demo<'a> {
    fn new() -> Self {
        let builder: [(&'static str, Box<dyn Fn() -> Element<'a, Message>>); 3] = [
            (
                "Cut Position",
                Box::new(|| widget::text("5432 px").size(24).into()),
            ),
            (
                "Page Preview",
                Box::new(|| widget::text("10870 px").size(24).into()),
            ),
            (
                "Page Setup",
                Box::new(|| widget::text("14102 px").size(24).into()),
            ),
        ];
        Self {
            pages: SingleSelectModelBuilder::from_array(builder)
                .with_first_as_active()
                .build(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectPage(id) => self.pages.activate(id),
            Message::Previous => {
                self.pages.activate_previous();
            }
            Message::Next => {
                self.pages.activate_next();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let first = self.pages.is_first();
        let last = self.pages.is_last();
        let pager = pager::horizontal(&self.pages).on_activate(Message::SelectPage);

        column![
            text("Cut position").size(12),
            (self.pages.active_data())(),
            pager,
            row![
                button("Previous").on_press_maybe((!first).then_some(Message::Previous)),
                button("Next").on_press_maybe((!last).then_some(Message::Next)),
            ]
            .spacing(8)
        ]
        .spacing(4)
        .apply(container)
        .padding(28)
        .center(Fill)
        .into()
    }
}
