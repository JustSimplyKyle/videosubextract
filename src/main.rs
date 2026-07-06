// SPDX-License-Identifier: MPL-2.0

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

    // create_video_player(input, crop_rectangle)

    // Starts the application's event loop with `()` as the application's flags.
    cosmic::app::run::<app::AppModel>(settings, ())
}

use std::{
    collections::VecDeque,
    fs::File,
    sync::{LazyLock, atomic::AtomicUsize},
    time::Duration,
};

use cosmic::iced;
use eyre::{Context, ContextCompat};
use ffmpeg_the_third::{
    self as ffmpeg, Packet, Stream,
    ffi::AV_TIME_BASE,
    filter::Graph,
    format::context::Input,
    frame::Video,
    media::{self},
    software::scaling::Flags,
};
use image::DynamicImage;
use ocr_rs::OcrEngine;
use opencv::imgcodecs;
use opencv::prelude::*;

use subfinder::{Params, SubtitleEvent};

use crate::subfinder::SubtitleSearch;

// --- example usage --------------------
// //
// fn main() -> eyre::Result<()> {
//     ffmpeg::init()?;
//     let mut input = ffmpeg::format::input("with_subtitle.mkv")?;

//     use cosmic::Apply;
//     let fps = input
//         .streams()
//         .best(Type::Video)
//         .ok_or(ffmpeg::Error::StreamNotFound)?
//         .apply(|x| x.avg_frame_rate())
//         .apply(f64::from);

//     skip(0, fps, &mut input, Duration::from_mins(6)).unwrap();

//     let (events, fps) = find_subtitles_in(
//         &mut input,
//         iced::Rectangle {
//             x: 0.,
//             y: 1080. - 128.,
//             width: 1920.,
//             height: 128.,
//         },
//     )?;

//     for (n, ev) in events.enumerate() {
//         println!(
//             "subtitle #{n}: {:.2}s -> {:.2}s",
//             ev.start_frame as f64 / fps,
//             ev.end_frame as f64 / fps,
//         );
//         let cleaned = ev.sample_bgr;
//         let image = &mat_to_dynamic_image(&cleaned)?;
//         let img = OCR.recognize(image)?;
//         println!(
//             "  text: {}",
//             img.first().map(|x| x.text.clone()).unwrap_or_default()
//         );
//     }
//     Ok(())
// }

static OCR: LazyLock<OcrEngine> = LazyLock::new(|| {
    OcrEngine::from_bytes(
        include_bytes!("../models/PP-OCRv5_mobile_det.mnn"),
        include_bytes!("../models/PP-OCRv5_mobile_rec.mnn"),
        include_bytes!("../models/ppocr_keys_v5.txt"),
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
