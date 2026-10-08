//! COSMIC semantic styles adapted to Iced 0.15's closure-based style API.
use ::iced::{Background, Border, Color};
use cosmic_theme::{Component, Theme as CosmicTheme, palette::Srgba};
use std::sync::LazyLock;

pub type Theme = ::iced::Theme;
pub static COSMIC: LazyLock<CosmicTheme> = LazyLock::new(CosmicTheme::dark_default);

/// A widget style with a border that can be customized.
pub trait WithRadius: Sized {
    /// Access the border to customize.
    fn border_mut(&mut self) -> &mut Border;

    /// Override the corner radius while preserving the rest of the style.
    fn with_radius(mut self, radius: impl Into<::iced::border::Radius>) -> Self {
        self.border_mut().radius = radius.into();
        self
    }
}

macro_rules! impl_with_radius {
    ($($widget:ident),+ $(,)?) => {
        $(impl WithRadius for ::iced::widget::$widget::Style {
            fn border_mut(&mut self) -> &mut Border {
                &mut self.border
            }
        })+
    };
}

impl_with_radius!(
    button,
    checkbox,
    container,
    pick_list,
    progress_bar,
    text_editor,
    text_input
);

/// Wrap a status-based style function with a corner-radius override.
///
/// The base style is evaluated for every status, preserving its colors and
/// other properties. Accepts a uniform radius or an [`::iced::border::Radius`]
/// with different values for each corner.
///
/// ```
/// use vse_ui::{theme, widget};
///
/// let button = widget::button(widget::text("Find subtitles"))
///     .on_press(())
///     .style(theme::with_radius(theme::button::suggested, 4.0));
/// ```
///
/// For styles without a status, such as containers, use [`WithRadius`] directly:
///
/// ```
/// use vse_ui::{theme::{self, WithRadius}, widget};
///
/// let container: widget::Container<'_, ()> = widget::container(widget::text("Content"))
///     .style(|theme| theme::container::card(theme).with_radius(4.0));
/// ```
pub fn with_radius<T, Status, Style: WithRadius>(
    style: impl Fn(&T, Status) -> Style,
    radius: impl Into<::iced::border::Radius>,
) -> impl Fn(&T, Status) -> Style {
    let radius = radius.into();
    move |theme, status| style(theme, status).with_radius(radius)
}

pub fn iced_theme() -> Theme {
    ::iced::Theme::custom(
        "COSMIC Dark",
        ::iced::theme::palette::Seed {
            background: color(COSMIC.background(false).base),
            text: color(COSMIC.background(false).on),
            primary: color(COSMIC.accent.base),
            success: color(COSMIC.success.base),
            warning: color(COSMIC.warning.base),
            danger: color(COSMIC.destructive.base),
        },
    )
}
pub fn color(value: Srgba) -> Color {
    Color::from_rgba(value.red, value.green, value.blue, value.alpha)
}
pub(crate) fn icon_color() -> Color {
    color(COSMIC.icon_button.on)
}
fn radius(value: [f32; 4]) -> ::iced::border::Radius {
    ::iced::border::Radius {
        top_left: value[0],
        top_right: value[1],
        bottom_right: value[2],
        bottom_left: value[3],
    }
}
fn button_style(
    component: &Component,
    radii: [f32; 4],
    status: ::iced::widget::button::Status,
) -> ::iced::widget::button::Style {
    use ::iced::widget::button::{Status, Style};
    let (background, foreground) = match status {
        Status::Active => (component.base, component.on),
        Status::Hovered => (component.hover, component.on),
        Status::Pressed => (component.pressed, component.on),
        Status::Disabled => (component.disabled, component.on_disabled),
    };
    Style {
        background: Some(Background::Color(color(background))),
        text_color: color(foreground),
        border: Border {
            radius: radius(radii),
            color: color(component.border),
            width: 0.0,
        },
        ..Style::default()
    }
}

fn navigation_item_style(
    selected: bool,
    status: ::iced::widget::button::Status,
) -> ::iced::widget::button::Style {
    use ::iced::widget::button::{Status, Style};

    let alpha = match status {
        Status::Active if selected => Some(0.2),
        Status::Hovered => Some(0.3),
        Status::Pressed => Some(0.25),
        Status::Active | Status::Disabled => None,
    };
    let background = alpha.map(|alpha| {
        let mut selection = COSMIC.palette.neutral_5;
        selection.alpha = alpha;
        Background::Color(color(selection))
    });

    Style {
        background,
        text_color: if selected || !matches!(status, Status::Active | Status::Disabled) {
            color(COSMIC.accent_text_color())
        } else {
            color(COSMIC.primary(false).component.on)
        },
        border: Border {
            radius: radius(COSMIC.corner_radii.radius_m),
            ..Border::default()
        },
        ..Style::default()
    }
}

pub mod button {
    use super::*;
    macro_rules! style {
        ($name:ident, $field:ident, $radius:ident) => {
            pub fn $name(
                _: &Theme,
                status: ::iced::widget::button::Status,
            ) -> ::iced::widget::button::Style {
                button_style(&COSMIC.$field, COSMIC.corner_radii.$radius, status)
            }
        };
    }
    style!(standard, button, radius_xl);
    style!(suggested, accent_button, radius_xl);
    style!(destructive, destructive_button, radius_xl);
    style!(icon, icon_button, radius_xl);
    style!(nav_toggle, icon_button, radius_s);

    pub fn navigation_active(
        _: &Theme,
        status: ::iced::widget::button::Status,
    ) -> ::iced::widget::button::Style {
        navigation_item_style(true, status)
    }

    pub fn navigation_inactive(
        _: &Theme,
        status: ::iced::widget::button::Status,
    ) -> ::iced::widget::button::Style {
        navigation_item_style(false, status)
    }
}

pub fn segmented_button(
    theme: &Theme,
    status: ::iced::widget::button::Status,
    active: bool,
    first: bool,
    last: bool,
) -> ::iced::widget::button::Style {
    let mut style = if active {
        button::suggested(theme, status)
    } else {
        button::standard(theme, status)
    };
    let radius = COSMIC.corner_radii.radius_s[0];
    style.border.radius = ::iced::border::Radius {
        top_left: if first { radius } else { 0.0 },
        bottom_left: if first { radius } else { 0.0 },
        top_right: if last { radius } else { 0.0 },
        bottom_right: if last { radius } else { 0.0 },
    };
    style
}

pub fn container_style(
    container: &cosmic_theme::Container,
    radii: [f32; 4],
) -> ::iced::widget::container::Style {
    ::iced::widget::container::Style {
        text_color: Some(color(container.on)),
        background: Some(Background::Color(color(container.base))),
        border: Border {
            radius: radius(radii),
            ..Border::default()
        },
        ..Default::default()
    }
}
pub mod container {
    use super::*;
    /// libcosmic's tooltip surface and corner radius.
    pub fn tooltip(_: &Theme) -> ::iced::widget::container::Style {
        ::iced::widget::container::Style {
            background: Some(Background::Color(color(COSMIC.palette.neutral_2))),
            border: Border {
                radius: radius(COSMIC.corner_radii.radius_l),
                ..Border::default()
            },
            ..Default::default()
        }
    }

    pub fn navigation(_: &Theme) -> ::iced::widget::container::Style {
        let surface = COSMIC.primary(false);
        container_style(surface, COSMIC.corner_radii.radius_s)
    }

    pub fn card(_: &Theme) -> ::iced::widget::container::Style {
        let layer = COSMIC.background(false);
        container_style(
            &cosmic_theme::Container {
                base: layer.component.base,
                component: layer.component.clone(),
                divider: layer.component.divider,
                on: layer.component.on,
                small_widget: layer.small_widget,
            },
            COSMIC.corner_radii.radius_s,
        )
    }

    pub fn secondary(_: &Theme) -> ::iced::widget::container::Style {
        container_style(COSMIC.secondary(false), COSMIC.corner_radii.radius_s)
    }

    pub fn settings_sidebar(_: &Theme) -> ::iced::widget::container::Style {
        container_style(COSMIC.primary(false), COSMIC.corner_radii.radius_l)
    }

    pub fn list(_: &Theme) -> ::iced::widget::container::Style {
        let layer = COSMIC.background(false);
        container_style(
            &cosmic_theme::Container {
                base: layer.component.base,
                component: layer.component.clone(),
                divider: layer.component.divider,
                on: layer.component.on,
                small_widget: layer.small_widget,
            },
            COSMIC.corner_radii.radius_s,
        )
    }

    pub fn dialog(_: &Theme) -> ::iced::widget::container::Style {
        let surface = COSMIC.primary(false);
        ::iced::widget::container::Style {
            text_color: Some(color(surface.on)),
            background: Some(Background::Color(color(surface.base))),
            border: Border {
                color: color(surface.divider),
                width: 1.0,
                radius: radius(COSMIC.corner_radii.radius_m),
            },
            shadow: ::iced::Shadow {
                color: color(COSMIC.shade),
                offset: ::iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..Default::default()
        }
    }
}

pub mod text {
    use super::*;
    pub fn accent(_: &Theme) -> ::iced::widget::text::Style {
        ::iced::widget::text::Style {
            color: Some(color(COSMIC.accent_text_color())),
        }
    }

    pub fn settings_label(_: &Theme) -> ::iced::widget::text::Style {
        ::iced::widget::text::Style {
            color: Some(color(COSMIC.palette.neutral_8)),
        }
    }
}

pub mod rule {
    use super::*;

    pub fn settings(_: &Theme) -> ::iced::widget::rule::Style {
        ::iced::widget::rule::Style {
            color: color(COSMIC.primary_container_divider()),
            radius: 0.0.into(),
            fill_mode: ::iced::widget::rule::FillMode::Full,
            snap: true,
        }
    }
}

pub mod text_input {
    use super::*;

    pub fn standard(
        _: &Theme,
        status: ::iced::widget::text_input::Status,
    ) -> ::iced::widget::text_input::Style {
        use ::iced::widget::text_input::{Status, Style};

        let field = &COSMIC.background(false).component;
        let border_color = match status {
            Status::Active => field.divider,
            Status::Hovered => field.on,
            Status::Focused { .. } => COSMIC.accent.base,
            Status::Disabled => field.disabled_border,
        };

        Style {
            background: Background::Color(color(match status {
                Status::Disabled => field.disabled,
                _ => COSMIC.bg_component_color(),
            })),
            border: Border {
                radius: radius(COSMIC.corner_radii.radius_s),
                width: 1.0,
                color: color(border_color),
            },
            placeholder: color(COSMIC.palette.neutral_7),
            value: color(match status {
                Status::Disabled => field.on_disabled,
                _ => field.on,
            }),
            selection: color(COSMIC.accent.base),
        }
    }
}

pub mod pick_list {
    use super::*;

    pub fn standard(
        _: &Theme,
        status: ::iced::widget::pick_list::Status,
    ) -> ::iced::widget::pick_list::Style {
        use ::iced::widget::pick_list::{Status, Style};

        let field = &COSMIC.background(false).component;
        let active = Style {
            text_color: color(field.on),
            placeholder_color: color(COSMIC.palette.neutral_7),
            handle_color: color(field.on),
            background: Background::Color(color(COSMIC.bg_component_color())),
            border: Border {
                radius: radius(COSMIC.corner_radii.radius_s),
                width: 1.0,
                color: color(field.divider),
            },
        };

        match status {
            Status::Active => active,
            Status::Hovered | Status::Opened { .. } => Style {
                border: Border {
                    color: color(COSMIC.accent.base),
                    ..active.border
                },
                ..active
            },
            Status::Disabled => Style {
                text_color: color(field.on_disabled),
                background: Background::Color(color(field.disabled)),
                border: Border {
                    color: color(field.disabled_border),
                    ..active.border
                },
                ..active
            },
        }
    }
}

pub mod toggler {
    use super::*;

    pub fn standard(
        _: &Theme,
        status: ::iced::widget::toggler::Status,
    ) -> ::iced::widget::toggler::Style {
        use ::iced::widget::toggler::{Status, Style};

        let (is_toggled, hovered, disabled) = match status {
            Status::Active { is_toggled } => (is_toggled, false, false),
            Status::Hovered { is_toggled } => (is_toggled, true, false),
            Status::Disabled { is_toggled } => (is_toggled, false, true),
        };
        let surface = COSMIC.primary(false);
        let component = if disabled {
            &surface.component
        } else if is_toggled {
            &COSMIC.accent
        } else {
            &surface.component
        };
        let background = if disabled {
            surface.component.disabled
        } else if hovered {
            component.hover
        } else if !is_toggled {
            surface.small_widget
        } else {
            component.base
        };

        Style {
            background: Background::Color(color(background)),
            background_border_width: 1.0,
            background_border_color: color(if disabled {
                surface.component.disabled_border
            } else {
                component.border
            }),
            foreground: Background::Color(color(if disabled {
                component.on_disabled
            } else {
                component.on
            })),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: None,
            border_radius: None,
            padding_ratio: 0.15,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Spacing {
    pub space_xxs: f32,
    pub space_xs: f32,
    pub space_s: f32,
    pub space_m: f32,
    pub space_l: f32,
    pub space_xl: f32,
}
pub fn spacing() -> Spacing {
    Spacing {
        space_xxs: COSMIC.space_xxs().into(),
        space_xs: COSMIC.space_xs().into(),
        space_s: COSMIC.space_s().into(),
        space_m: COSMIC.space_m().into(),
        space_l: COSMIC.space_l().into(),
        space_xl: COSMIC.space_xl().into(),
    }
}
pub mod iced {
    #[derive(Debug, Clone, Copy, Default)]
    pub struct TextEditor;
}
