//! Upstream Iced widget re-exports and the home for COSMIC-look components.

pub use iced::widget::*;
pub use iced::widget::{canvas, image, text_editor};

/// Creates a button using the default COSMIC button appearance.
///
/// This intentionally shadows Iced's unstyled `button` helper while keeping
/// the rest of `iced::widget` re-exported from this module. Call `.style(...)`
/// on the returned button to select a different semantic variant.
pub fn button<'a, Message>(
    content: impl Into<crate::Element<'a, Message>>,
) -> iced::widget::Button<'a, Message, crate::Theme> {
    iced::widget::button(content).style(crate::theme::button::standard)
}

pub mod icon {
    use iced::{Element, Length};

    pub fn from_name<'a, Message: 'a>(name: impl AsRef<str>) -> Element<'a, Message> {
        let name = name.as_ref();
        let symbolic = name.ends_with("-symbolic");
        #[cfg(all(unix, not(target_os = "macos")))]
        let path = {
            let extra_paths = std::env::var_os("COSMIC_ICONS")
                .map(std::path::PathBuf::from)
                .into_iter()
                .collect::<Vec<_>>();
            let lookup = |candidate: &str| {
                let lookup = freedesktop_icons::lookup(candidate)
                    .with_theme("Cosmic")
                    .with_extra_paths(&extra_paths)
                    .with_size(16)
                    .with_cache();
                if symbolic {
                    lookup.force_svg().find()
                } else {
                    lookup.find()
                }
            };

            lookup(name).or_else(|| {
                name.rmatch_indices('-')
                    .find_map(|(position, _)| lookup(&name[..position]))
            })
        };
        #[cfg(any(not(unix), target_os = "macos"))]
        let path: Option<std::path::PathBuf> = None;

        match path {
            Some(path) => iced::widget::svg(iced::widget::svg::Handle::from_path(path))
                // This upstream Iced revision cannot inherit a button's icon
                // color. Use the COSMIC icon component foreground instead.
                .style(move |_, _| iced::widget::svg::Style {
                    color: symbolic.then(crate::theme::icon_color),
                })
                .width(Length::Fixed(16.0))
                .height(Length::Fixed(16.0))
                .into(),
            None => iced::widget::space().width(16).height(16).into(),
        }
    }
}

pub fn dropdown<'a, Message: Clone + 'a>(
    options: Vec<impl Into<String>>,
    selected: Option<usize>,
    on_select: impl Fn(usize) -> Message + 'a,
) -> iced::widget::PickList<'a, String, Vec<String>, String, Message> {
    let options = options.into_iter().map(Into::into).collect::<Vec<_>>();
    let selected = selected.and_then(|index| options.get(index).cloned());
    let indexes = options.clone();
    iced::widget::pick_list(selected, options, String::clone).on_select(move |value| {
        on_select(
            indexes
                .iter()
                .position(|candidate| candidate == &value)
                .unwrap_or_default(),
        )
    })
}

/// A compact value control with decrement and increment buttons.
pub fn spin_button<'a, T, Message>(
    label: impl Into<std::borrow::Cow<'a, str>>,
    _name: impl Into<std::borrow::Cow<'a, str>>,
    value: T,
    step: T,
    min: T,
    max: T,
    on_press: impl Fn(T) -> Message + 'a,
) -> iced::Element<'a, Message>
where
    T: Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T> + PartialOrd + 'a,
    Message: Clone + 'a,
{
    let value = if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    };
    let decrement = if value < min + step {
        min
    } else {
        value - step
    };
    let increment = if value > max - step {
        max
    } else {
        value + step
    };

    let decrement_button = button(icon::from_name("list-remove-symbolic"))
        .padding(6)
        .style(crate::theme::button::icon);
    let decrement_button = if value > min {
        decrement_button.on_press(on_press(decrement))
    } else {
        decrement_button
    };

    let increment_button = button(icon::from_name("list-add-symbolic"))
        .padding(6)
        .style(crate::theme::button::icon);
    let increment_button = if value < max {
        increment_button.on_press(on_press(increment))
    } else {
        increment_button
    };

    iced::widget::row![
        decrement_button,
        iced::widget::container(iced::widget::text(label.into()))
            .center_x(48)
            .center_y(28),
        increment_button,
    ]
    .align_y(iced::Alignment::Center)
    .into()
}

pub mod segmented_button {
    use std::any::Any;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Entity(pub(crate) u64);

    pub struct SingleSelectModel {
        next: u64,
        active: Option<Entity>,
        entries: Vec<Entry>,
    }

    struct Entry {
        id: Entity,
        text: String,
        data: Box<dyn Any>,
    }

    impl Default for SingleSelectModel {
        fn default() -> Self {
            Self {
                next: 0,
                active: None,
                entries: Vec::new(),
            }
        }
    }

    impl SingleSelectModel {
        pub fn insert(&mut self) -> Inserter<'_> {
            Inserter {
                model: self,
                text: String::new(),
            }
        }
        pub fn activate(&mut self, id: Entity) {
            if self.entries.iter().any(|entry| entry.id == id) {
                self.active = Some(id);
            }
        }
        pub fn active(&self) -> Option<Entity> {
            self.active
        }
        pub fn active_data<T: 'static>(&self) -> Option<&T> {
            self.entries
                .iter()
                .find(|entry| Some(entry.id) == self.active)?
                .data
                .downcast_ref()
        }
        pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
            self.entries.iter().map(|entry| entry.id)
        }
        pub fn text_set(&mut self, id: Entity, text: impl Into<String>) {
            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
                entry.text = text.into();
            }
        }
    }

    pub struct Inserter<'a> {
        model: &'a mut SingleSelectModel,
        text: String,
    }
    impl<'a> Inserter<'a> {
        pub fn text(mut self, text: impl Into<String>) -> Self {
            self.text = text.into();
            self
        }
        pub fn data<T: 'static>(self, data: T) -> Inserted {
            let id = Entity(self.model.next);
            self.model.next += 1;
            self.model.entries.push(Entry {
                id,
                text: self.text,
                data: Box::new(data),
            });
            Inserted(id)
        }
    }
    pub struct Inserted(Entity);
    impl Inserted {
        pub fn id(self) -> Entity {
            self.0
        }
    }

    pub(crate) fn labels(
        model: &SingleSelectModel,
    ) -> impl Iterator<Item = (Entity, String, bool)> + '_ {
        model
            .entries
            .iter()
            .map(|entry| (entry.id, entry.text.clone(), model.active == Some(entry.id)))
    }
}

pub mod segmented_control {
    use crate::Apply;

    use super::segmented_button::{self, Entity, SingleSelectModel};
    use iced::widget::{Component, button, component, row, text};
    use iced::{Element, Renderer};

    pub fn horizontal(model: &SingleSelectModel) -> SegmentedControl<'_> {
        SegmentedControl { model }
    }

    pub struct SegmentedControl<'a> {
        model: &'a SingleSelectModel,
    }
    pub struct ActiveSegmentedControl<'a, Message> {
        model: &'a SingleSelectModel,
        on_activate: crate::components::MessageEmitter<'a, Entity, Message>,
    }

    impl<'a> SegmentedControl<'a> {
        pub fn on_activate<Message>(
            self,
            f: impl Fn(Entity) -> Message + 'a,
        ) -> ActiveSegmentedControl<'a, Message> {
            ActiveSegmentedControl {
                model: self.model,
                on_activate: Box::new(f),
            }
        }
    }

    impl<'a, Message: 'a> Component<'a, Message> for ActiveSegmentedControl<'a, Message> {
        type State = ();
        type Event = Entity;

        fn update(&mut self, _: &mut Self::State, event: Entity, _: &Renderer) -> Option<Message> {
            Some((self.on_activate)(event))
        }

        fn view(&self, _: &Self::State) -> Element<'a, Self::Event> {
            let labels = segmented_button::labels(self.model).collect::<Vec<_>>();
            let last = labels.len().saturating_sub(1);
            labels
                .into_iter()
                .enumerate()
                .map(|(index, (id, label, active))| {
                    button(text(label))
                        .on_press(id)
                        .padding([6, 14])
                        .style(move |theme, status| {
                            crate::theme::segmented_button(
                                theme,
                                status,
                                active,
                                index == 0,
                                index == last,
                            )
                        })
                        .into()
                })
                .apply(row)
                .spacing(1)
                .into()
        }
    }

    impl<'a, Message: 'a> From<ActiveSegmentedControl<'a, Message>> for Element<'a, Message> {
        fn from(control: ActiveSegmentedControl<'a, Message>) -> Self {
            component(control)
        }
    }
}

/// COSMIC's semantic text vocabulary, implemented with upstream Iced text.
pub mod text {
    pub use iced::widget::text;
    pub use iced::widget::text::*;

    pub fn title1<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
        text(content).size(28)
    }

    pub fn title3<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
        text(content).size(20)
    }

    pub fn body<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
        text(content).size(16)
    }

    pub fn caption<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
        text(content).size(12)
    }

    pub fn monotext<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
        text(content).font(iced::Font::MONOSPACE)
    }
}

/// Settings layouts and controls. `ToggleItem` is intentionally a Component:
/// it owns its interaction event and only emits the parent message at its edge.
pub mod settings;
