// SPDX-License-Identifier: MPL-2.0

use cosmic::iced_anim::Motion;

mod app;
pub mod apply_traits;
mod cli;
mod config;
mod extraction;
mod i18n;
mod native_video_sub_finder;
mod ocr;
mod subfinder;

pub mod video_player;

fn main() -> eyre::Result<()> {
    use clap::Parser;

    let args = cli::Args::parse();

    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();
    i18n::init(&requested_languages);

    assert_eq!(
        native_video_sub_finder::api_version(),
        native_video_sub_finder::EXPECTED_API_VERSION,
        "incompatible VideoSubFinder native library"
    );

    if let Some(input) = args.input {
        let output = args.output.unwrap_or_else(|| input.with_extension("srt"));
        return smol::block_on(cli::run(input, output, args.crop));
    }

    run_gui()?;
    Ok(())
}

fn run_gui() -> cosmic::iced::Result {
    let mut frosted_theme = cosmic::cosmic_theme::Theme::dark_default();
    frosted_theme.frosted = cosmic::cosmic_theme::BlurStrength::VeryHigh2;

    let mut theme = cosmic::Theme::custom(std::sync::Arc::new(frosted_theme));
    theme.transparent = true;

    let settings = cosmic::app::Settings::default()
        .theme(theme)
        .size_limits(
            cosmic::iced::Limits::NONE
                .min_width(360.0)
                .min_height(180.0),
        )
        .animation(Motion::SMOOTH)
        .nav_bar_content_transition(Motion::SMOOTH);

    // Starts the application's event loop with `()` as the application's flags.
    cosmic::app::run::<app::AppModel>(settings, ())
}
