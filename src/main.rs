// SPDX-License-Identifier: MPL-2.0

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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        return runtime.block_on(cli::run(input, output, args.crop));
    }

    run_gui()?;
    Ok(())
}

fn run_gui() -> iced::Result {
    iced::application(
        app::AppModel::boot,
        app::AppModel::update,
        app::AppModel::view,
    )
    .title(app::AppModel::title)
    .subscription(app::AppModel::subscription)
    .theme(app::AppModel::theme)
    .window_size((1100.0, 760.0))
    .run()
}
