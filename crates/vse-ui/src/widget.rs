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
    use indexmap::IndexMap;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Entity(pub(crate) u64);

    pub struct SingleSelectModel<T> {
        entries: IndexMap<Entity, Entry<T>>,
        active: Entity,
    }

    /// The builder has no entries and no active selection yet.
    pub struct MissingActiveWithoutEntry;

    /// The builder has entries, but none has been selected as active yet.
    pub struct MissingActiveWithEntries {
        first: Entity,
    }

    /// The builder has an active entry and can produce a model.
    pub struct HasActive {
        active: Entity,
    }

    /// Builds a [`SingleSelectModel`] with at least one active entry.
    ///
    /// The `State` parameter tracks construction: [`build`](Self::build) is
    /// available only after an entry is selected with [`active`](Self::active)
    /// or [`with_first_as_active`](Self::with_first_as_active).
    ///
    /// For a fixed set of entries, [`from_array`](Self::from_array) checks at
    /// compile time that the array is nonempty:
    ///
    /// ```
    /// use vse_ui::widget::segmented_button::SingleSelectModelBuilder;
    ///
    /// let model = SingleSelectModelBuilder::from_array([
    ///     ("Original", 0),
    ///     ("Converted", 1),
    /// ])
    /// .with_first_as_active()
    /// .build();
    /// assert_eq!(*model.active_data(), 0);
    /// ```
    ///
    /// Entries can also be added one at a time:
    ///
    /// ```
    /// use vse_ui::widget::segmented_button::SingleSelectModelBuilder;
    ///
    /// let model = SingleSelectModelBuilder::new()
    ///     .insert("Original", 0)
    ///     .insert("Converted", 1)
    ///     .with_first_as_active()
    ///     .build();
    /// assert_eq!(*model.active_data(), 0);
    /// ```
    pub struct SingleSelectModelBuilder<T, State = MissingActiveWithoutEntry> {
        next: u64,
        entries: IndexMap<Entity, Entry<T>>,
        state: State,
    }

    impl<T> SingleSelectModelBuilder<T, MissingActiveWithoutEntry> {
        /// Creates an empty builder. Add an active entry before calling `build`.
        pub fn new() -> Self {
            Self {
                next: 0,
                entries: IndexMap::new(),
                state: MissingActiveWithoutEntry,
            }
        }

        /// Adds the first entry and marks it active.
        #[must_use]
        pub fn active(
            mut self,
            text: impl Into<String>,
            data: T,
        ) -> SingleSelectModelBuilder<T, HasActive> {
            let id = Entity(self.next);
            self.next += 1;
            self.entries.shift_insert(
                0,
                id,
                Entry {
                    text: text.into(),
                    data,
                },
            );
            SingleSelectModelBuilder {
                next: self.next,
                entries: self.entries,
                state: HasActive { active: id },
            }
        }

        /// Adds a fixed, nonempty array of `(label, data)` pairs in order.
        /// Call [`with_first_as_active`](Self::with_first_as_active) to select
        /// its first entry before building the model.
        ///
        /// An empty array fails to compile:
        ///
        /// ```compile_fail
        /// use vse_ui::widget::segmented_button::SingleSelectModelBuilder;
        ///
        /// let _ = SingleSelectModelBuilder::<u8>::from_array([] as [(String, u8); 0]);
        /// ```
        pub fn from_array<S: Into<String>, const N: usize>(
            items: [(S, T); N],
        ) -> SingleSelectModelBuilder<T, MissingActiveWithEntries> {
            const { assert!(N > 0, "single select models need at least one entry") };

            let mut builder = Self::new();
            for (text, data) in items {
                let id = Entity(builder.next);
                builder.next += 1;
                builder.entries.insert(
                    id,
                    Entry {
                        text: text.into(),
                        data,
                    },
                );
            }
            SingleSelectModelBuilder {
                next: builder.next,
                entries: builder.entries,
                state: MissingActiveWithEntries { first: Entity(0) },
            }
        }
    }

    impl<T> SingleSelectModelBuilder<T, MissingActiveWithEntries> {
        /// Selects the first inserted entry as active.
        #[must_use]
        pub fn with_first_as_active(self) -> SingleSelectModelBuilder<T, HasActive> {
            SingleSelectModelBuilder {
                next: self.next,
                entries: self.entries,
                state: HasActive {
                    active: self.state.first,
                },
            }
        }
    }

    impl<T> Default for SingleSelectModelBuilder<T, MissingActiveWithoutEntry> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl<T> SingleSelectModelBuilder<T, MissingActiveWithoutEntry> {
        /// Adds the first entry without selecting it yet.
        #[must_use]
        pub fn insert(
            mut self,
            text: impl Into<String>,
            data: T,
        ) -> SingleSelectModelBuilder<T, MissingActiveWithEntries> {
            let id = Entity(self.next);
            self.next += 1;
            self.entries.insert(
                id,
                Entry {
                    text: text.into(),
                    data,
                },
            );
            SingleSelectModelBuilder {
                next: self.next,
                entries: self.entries,
                state: MissingActiveWithEntries { first: id },
            }
        }
    }
    impl<T> SingleSelectModelBuilder<T, MissingActiveWithEntries> {
        /// Appends another entry while leaving the first entry available for selection.
        #[must_use]
        pub fn insert(mut self, text: impl Into<String>, data: T) -> Self {
            let id = Entity(self.next);
            self.next += 1;
            self.entries.insert(
                id,
                Entry {
                    text: text.into(),
                    data,
                },
            );
            self
        }
    }

    impl<T> SingleSelectModelBuilder<T, HasActive> {
        /// Appends an entry without changing the active selection.
        #[must_use]
        pub fn insert(mut self, text: impl Into<String>, data: T) -> Self {
            let id = Entity(self.next);
            self.next += 1;
            self.entries.insert(
                id,
                Entry {
                    text: text.into(),
                    data,
                },
            );
            self
        }

        /// Finishes the builder with its selected active entry.
        pub fn build(self) -> SingleSelectModel<T> {
            SingleSelectModel {
                entries: self.entries,
                active: self.state.active,
            }
        }
    }

    pub struct Entry<T> {
        text: String,
        data: T,
    }

    impl<T> Entry<T> {
        pub fn text(&self) -> &str {
            &self.text
        }
        pub const fn data(&self) -> &T {
            &self.data
        }
    }

    impl<T> SingleSelectModel<T> {
        pub fn activate(&mut self, id: Entity) {
            if self.entries.contains_key(&id) {
                self.active = id;
            }
        }
        pub const fn active(&self) -> Entity {
            self.active
        }
        pub fn active_data(&self) -> &T {
            &self.entries[&self.active].data
        }
        pub fn text_set(&mut self, id: Entity, text: impl Into<String>) {
            self.entries[&id].text = text.into();
        }
        pub const fn entries(&self) -> &IndexMap<Entity, Entry<T>> {
            &self.entries
        }
    }
}

pub mod segmented_control {
    use crate::Apply;

    use super::segmented_button::{Entity, SingleSelectModel};
    use iced::widget::{Component, button, component, row, text};
    use iced::{Element, Renderer};

    pub fn horizontal<T>(model: &SingleSelectModel<T>) -> SegmentedControlBuilder<'_, T> {
        SegmentedControlBuilder { model }
    }

    pub struct SegmentedControlBuilder<'a, T> {
        model: &'a SingleSelectModel<T>,
    }
    pub struct SegmentedControl<'a, Message, T> {
        model: &'a SingleSelectModel<T>,
        on_activate: crate::components::MessageEmitter<'a, Entity, Message>,
    }

    impl<'a, T> SegmentedControlBuilder<'a, T> {
        pub fn on_activate<Message>(
            self,
            f: impl Fn(Entity) -> Message + 'a,
        ) -> SegmentedControl<'a, Message, T> {
            SegmentedControl {
                model: self.model,
                on_activate: Box::new(f),
            }
        }
    }

    impl<'a, Message: 'a, T> Component<'a, Message> for SegmentedControl<'a, Message, T> {
        type State = ();
        type Event = Entity;

        fn update(&mut self, _: &mut Self::State, event: Entity, _: &Renderer) -> Option<Message> {
            Some((self.on_activate)(event))
        }

        fn view(&self, _: &Self::State) -> Element<'a, Self::Event> {
            let labels = self.model.entries();
            let active = self.model.active();
            let last = labels.len().saturating_sub(1);
            labels
                .into_iter()
                .enumerate()
                .map(|(index, (id, entry))| {
                    button(text(entry.text()))
                        .on_press(*id)
                        .padding([6, 14])
                        .style(move |theme, status| {
                            crate::theme::segmented_button(
                                theme,
                                status,
                                active == *id,
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

    impl<'a, Message: 'a, T> From<SegmentedControl<'a, Message, T>> for Element<'a, Message> {
        fn from(control: SegmentedControl<'a, Message, T>) -> Self {
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
