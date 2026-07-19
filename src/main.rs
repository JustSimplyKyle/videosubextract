// SPDX-License-Identifier: MPL-2.0

mod app;
pub mod apply_traits;
mod config;
mod i18n;
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

    // Settings for configuring the application window and iced runtime.
    let settings = cosmic::app::Settings::default().size_limits(
        cosmic::iced::Limits::NONE
            .min_width(360.0)
            .min_height(180.0),
    );

    // Starts the application's event loop with `()` as the application's flags.
    cosmic::app::run::<app::AppModel>(settings, ())
}
