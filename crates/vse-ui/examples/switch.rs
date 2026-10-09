//! Run with `cargo run -p vse-ui --example switch`.
use iced::widget::{column, container, pick_list, row, text};
use iced::{Element, Fill, Task};
use vse_ui::motion::{SpringConfig, presets};
use vse_ui::widget::{AnimatedDropdown, button, switch};

fn main() -> iced::Result {
    iced::application(Demo::default, Demo::update, Demo::view)
        .title("Spring switch")
        .theme(|_: &Demo| vse_ui::theme::iced_theme())
        .window_size((440, 320))
        .run()
}

#[derive(Default)]
struct Demo {
    enabled: bool,
    preset: Preset,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Preset {
    #[default]
    Gentle,
    Quick,
    Bouncy,
    Slow,
}

impl Preset {
    const ALL: [Self; 4] = [Self::Gentle, Self::Quick, Self::Bouncy, Self::Slow];

    fn config(self) -> SpringConfig {
        match self {
            Self::Gentle => presets::gentle(),
            Self::Quick => presets::quick(),
            Self::Bouncy => presets::bouncy(),
            Self::Slow => presets::slow(),
        }
    }
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Gentle => "Gentle",
            Self::Quick => "Quick",
            Self::Bouncy => "Bouncy",
            Self::Slow => "Slow",
        })
    }
}

#[derive(Debug, Clone)]
enum Message {
    Changed(bool),
    Flip,
    PresetSelected(Preset),
}

impl Demo {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Changed(value) => self.enabled = value,
            Message::Flip => self.enabled = !self.enabled,
            Message::PresetSelected(preset) => self.preset = preset,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        container(
            column![
                text("Spring switch").size(24),
                row![
                    text("Spring preset").width(Fill),
                    AnimatedDropdown::from(
                        pick_list(Some(self.preset), Preset::ALL, Preset::to_string)
                            .on_select(Message::PresetSelected),
                    )
                    .with_config(self.preset.config()),
                ]
                .align_y(iced::Alignment::Center),
                row![
                    text(if self.enabled { "On" } else { "Off" }).width(Fill),
                    switch(self.enabled)
                        .with_config(self.preset.config())
                        .on_toggle(Message::Changed),
                ]
                .align_y(iced::Alignment::Center),
                button("Toggle from the parent").on_press(Message::Flip),
                row![text("Disabled").width(Fill), switch(true)].align_y(iced::Alignment::Center),
                text("Toggle rapidly to interrupt and reverse the spring.").size(13),
            ]
            .spacing(16),
        )
        .padding(28)
        .center(Fill)
        .into()
    }
}
