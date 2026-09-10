//! COSMIC semantic styles adapted to Iced 0.15's closure-based style API.
use ::iced::{Background, Border, Color};
use cosmic_theme::{Component, Theme as CosmicTheme, palette::Srgba};
use std::sync::LazyLock;

pub type Theme = ::iced::Theme;
static COSMIC: LazyLock<CosmicTheme> = LazyLock::new(CosmicTheme::dark_default);

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
fn color(value: Srgba) -> Color {
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

#[allow(non_snake_case)]
pub mod Button {
    use super::*;
    macro_rules! style {
        ($name:ident, $field:ident, $radius:ident) => {
            pub struct $name;
            impl $name {
                pub fn style(
                    _: &Theme,
                    status: ::iced::widget::button::Status,
                ) -> ::iced::widget::button::Style {
                    button_style(&COSMIC.$field, COSMIC.corner_radii.$radius, status)
                }
            }
        };
    }
    style!(Standard, button, radius_xl);
    style!(Suggested, accent_button, radius_xl);
    style!(Destructive, destructive_button, radius_xl);
    style!(Icon, icon_button, radius_xl);
    style!(NavToggle, icon_button, radius_s);

    pub struct NavigationActive;
    impl NavigationActive {
        pub fn style(
            _: &Theme,
            status: ::iced::widget::button::Status,
        ) -> ::iced::widget::button::Style {
            navigation_item_style(true, status)
        }
    }

    pub struct NavigationInactive;
    impl NavigationInactive {
        pub fn style(
            _: &Theme,
            status: ::iced::widget::button::Status,
        ) -> ::iced::widget::button::Style {
            navigation_item_style(false, status)
        }
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
        Button::Suggested::style(theme, status)
    } else {
        Button::Standard::style(theme, status)
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

fn container_style(
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
#[allow(non_snake_case)]
pub mod Container {
    use super::*;
    pub struct Navigation;
    impl Navigation {
        pub fn style(_: &Theme) -> ::iced::widget::container::Style {
            let surface = COSMIC.primary(false);
            container_style(surface, COSMIC.corner_radii.radius_s)
        }
    }
    pub struct Card;
    impl Card {
        pub fn style(_: &Theme) -> ::iced::widget::container::Style {
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
    }
    pub struct Secondary;
    impl Secondary {
        pub fn style(_: &Theme) -> ::iced::widget::container::Style {
            container_style(COSMIC.secondary(false), COSMIC.corner_radii.radius_s)
        }
    }
    pub struct List;
    impl List {
        pub fn style(_: &Theme) -> ::iced::widget::container::Style {
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
    }
    pub struct Dialog;
    impl Dialog {
        pub fn style(_: &Theme) -> ::iced::widget::container::Style {
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
}
#[allow(non_snake_case)]
pub mod Text {
    use super::*;
    pub struct Accent;
    impl Accent {
        pub fn style(_: &Theme) -> ::iced::widget::text::Style {
            ::iced::widget::text::Style {
                color: Some(color(COSMIC.accent_text_color())),
            }
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
