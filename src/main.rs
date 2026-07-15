// SPDX-License-Identifier: MPL-2.0

use std::sync::LazyLock;

use ocr_rs::OcrEngine;

mod app;
mod config;
mod i18n;
pub mod subfinder;

pub mod video_player;

fn main() -> cosmic::iced::Result {
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

static OCR: LazyLock<OcrEngine> = LazyLock::new(|| {
    OcrEngine::from_bytes(
        include_bytes!("../models/PP-OCRv6_medium_det.mnn"),
        include_bytes!("../models/PP-OCRv6_medium_rec.mnn"),
        include_bytes!("../models/ppocr_keys_v6_medium.txt"),
        Some(ocr_rs::OcrEngineConfig {
            det_options: ocr_rs::DetOptions {
                ..Default::default()
            },
            rec_options: ocr_rs::RecOptions {
                ..Default::default()
            },
            enable_parallel: true,
            backend: ocr_rs::Backend::Vulkan,
            ..Default::default()
        }),
    )
    .unwrap()
});
