// SPDX-License-Identifier: MPL-2.0

mod app;
pub mod apply_traits;
mod config;
mod i18n;
mod icons;
mod native_video_sub_finder;
mod ocr;
mod subfinder;

pub mod video_player;

fn main() -> cosmic::iced::Result {
    assert_eq!(
        native_video_sub_finder::api_version(),
        native_video_sub_finder::EXPECTED_API_VERSION,
        "incompatible VideoSubFinder native library"
    );

    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    let mut frosted_theme = cosmic::cosmic_theme::Theme::dark_default();
    frosted_theme.frosted = cosmic::cosmic_theme::BlurStrength::VeryHigh2;

    let mut theme = cosmic::Theme::custom(std::sync::Arc::new(frosted_theme));
    theme.transparent = true;

    let settings = cosmic::app::Settings::default().theme(theme).size_limits(
        cosmic::iced::Limits::NONE
            .min_width(360.0)
            .min_height(180.0),
    );

    // Starts the application's event loop with `()` as the application's flags.
    cosmic::app::run::<app::AppModel>(settings, ())
}
