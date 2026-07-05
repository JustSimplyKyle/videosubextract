// SPDX-License-Identifier: MPL-2.0

mod app;
mod config;
mod i18n;
pub mod subfinder;

// fn main() -> cosmic::iced::Result {
//     // Get the system's preferred languages.
//     let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

//     // Enable localizations to be applied.
//     i18n::init(&requested_languages);

//     // Settings for configuring the application window and iced runtime.
//     let settings = cosmic::app::Settings::default().size_limits(
//         cosmic::iced::Limits::NONE
//             .min_width(360.0)
//             .min_height(180.0),
//     );

//     // Starts the application's event loop with `()` as the application's flags.
//     cosmic::app::run::<app::AppModel>(settings, ())
// }

use std::{fs::File, sync::LazyLock, time::Duration};

use eyre::{Context, ContextCompat};
use ffmpeg_the_third::{
    self as ffmpeg, Packet, Stream, codec,
    ffi::AV_TIME_BASE,
    format::{Pixel, context::Input},
    frame::Video,
    media,
    software::scaling::Flags,
    threading,
};
use image::DynamicImage;
use ocr_rs::OcrEngine;
use opencv::prelude::*;
use opencv::{
    core::{CV_8UC3, Mat, Scalar},
    imgcodecs,
};

use subfinder::{Params, SubtitleEvent};

use crate::subfinder::SubtitleSearch;

/// Same decoder/threading/scaler setup as `video_processing::decode_video`, except
/// the scaler target is `Pixel::BGR24` (OpenCV's native layout) at the source
/// resolution, and each decoded frame becomes an `opencv::core::Mat` instead of a
/// `Texture2D`. No `retain_aspect_ratio_scale` step — that's a display-only concern.
fn decode_video_to_mats<'a, T: Iterator<Item = (Stream<'a>, Packet)>>(
    video_packets: T,
) -> eyre::Result<(impl Iterator<Item = Mat> + use<'a, T>, f64)> {
    let mut video_packets = video_packets.peekable();

    let (avg_frame_rate, vstream) = video_packets
        .peek()
        .map(|x| (x.0.avg_frame_rate().into(), x.0.parameters()))
        .context("not possible")?;

    let mut vcodec = codec::context::Context::from_parameters(vstream)?;
    if let Ok(parallelism) = std::thread::available_parallelism() {
        vcodec.set_threading(threading::Config {
            kind: threading::Type::Frame,
            count: parallelism.get(),
        });
    }

    let mut vdecoder = vcodec.decoder().video()?;
    let (width, height) = (vdecoder.width() as i32, vdecoder.height() as i32);

    let mut scaler = ffmpeg::software::scaling::Context::get(
        vdecoder.format(),
        vdecoder.width(),
        vdecoder.height(),
        Pixel::BGR24,
        vdecoder.width(),
        vdecoder.height(),
        Flags::BILINEAR,
    )?;

    let frames = video_packets
        .map(|x| x.1)
        .filter_map(move |packet| {
            unsafe {
                if packet.is_empty() {
                    vdecoder.send_eof().ok()?;
                } else {
                    vdecoder.send_packet(&packet).ok()?;
                }
            }
            let mut decoded_video = Video::empty();
            let mut mats = Vec::new();
            while vdecoder.receive_frame(&mut decoded_video).is_ok() {
                let mut bgr_frame = Video::empty();
                scaler.run(&decoded_video, &mut bgr_frame).ok()?;
                mats.push(video_frame_to_mat(&bgr_frame, width, height).ok()?);
            }
            Some(mats)
        })
        .flatten();

    Ok((frames, avg_frame_rate))
}

/// Copy a BGR24 ffmpeg frame into an owned `Mat`. A copy (not a zero-copy wrap) is
/// necessary because `bgr_frame` is dropped at the end of each loop iteration above;
/// it's done row-by-row since ffmpeg's `linesize`/stride is often wider than
/// `width * 3` (alignment padding), so it can't be treated as one contiguous slice.
fn video_frame_to_mat(frame: &Video, width: i32, height: i32) -> eyre::Result<Mat> {
    let stride = frame.stride(0);
    let data = frame.data(0);

    let mut mat = Mat::new_rows_cols_with_default(height, width, CV_8UC3, Scalar::all(0.0))?;
    for y in 0..height as usize {
        let row_start = y * stride;
        let row_end = row_start + (width as usize * 3);
        let mut dst = mat.row_mut(y as i32)?;
        dst.data_bytes_mut()?
            .copy_from_slice(&data[row_start..row_end]);
    }
    Ok(mat)
}

/// The subtitle-search equivalent of `video_processing::get_video_player`: pulls
/// the video stream out of `input`, decodes it straight to `Mat` frames, and runs
/// the whole pipeline via `subfinder::search_subtitles`.
pub fn find_subtitles_in(
    input: &mut Input,
) -> eyre::Result<(impl Iterator<Item = SubtitleEvent>, f64)> {
    let vstream_id = input
        .streams()
        .best(media::Type::Video)
        .context("stream not found")?
        .index();

    let packets = input.packets().filter_map(Result::ok);
    let video_packets = packets.filter(move |x| x.0.index() == vstream_id);

    let (frames, frame_rate) = decode_video_to_mats(video_packets)?;

    let mut params = Params::default();
    params.roi = opencv::core::Rect_ {
        x: 0,
        y: 1080 - 128,
        width: 1920,
        height: 128,
    };
    //     for event in SubtitleSearch::new(frames, params) {
    //         println!("subtitle frames [{}, {}]", event.start_frame, event.end_frame);
    //         // event.sample_bgr / event.mask feed into clean_for_ocr(...)
    //     }

    let events = SubtitleSearch::new(frames, params);

    Ok((events, frame_rate))
}

fn mat_to_dynamic_image(mat: &Mat) -> eyre::Result<DynamicImage> {
    use opencv::core::Vector;

    let mut buf = Vector::<u8>::new();
    imgcodecs::imencode(".png", mat, &mut buf, &Vector::new())
        .map_err(|e| eyre::eyre!("imencode failed: {e}"))?;

    image::load_from_memory(buf.as_slice()).context("decoding encoded Mat back to an image")
}

fn skip(
    current_frame: usize,
    frame_rate: f64,
    input: &mut Input,
    duration: Duration,
) -> eyre::Result<()> {
    let current_us = (current_frame as f64 / frame_rate * AV_TIME_BASE as f64) as i64;
    let target_us = current_us + (duration.as_secs_f64() * AV_TIME_BASE as f64) as i64;

    // min_ts = i64::MIN, max_ts = target_us: land on the nearest keyframe
    // at-or-before target_us (seeking can't land mid-GOP without full re-decode).
    input.seek(target_us, i64::MIN..=target_us)?;
    Ok(())
}

// --- example usage, same shape as video_processing::main --------------------
//
fn main() -> eyre::Result<()> {
    ffmpeg::init()?;
    let mut input = ffmpeg::format::input("with_subtitle.mkv")?;

    skip(0, 24.0, &mut input, Duration::from_mins(6)).unwrap();

    let (events, fps) = find_subtitles_in(&mut input)?;

    for (n, ev) in events.enumerate() {
        let start = ev.start_frame as f64 / fps;
        println!(
            "subtitle #{n}: {:.2}s -> {:.2}s",
            ev.start_frame as f64 / fps,
            ev.end_frame as f64 / fps,
        );
        let cleaned = ev.sample_bgr;
        let image = &mat_to_dynamic_image(&cleaned)?;
        let f = File::create_new(format!("image{}-{:.2}.png", n, start))?;
        image.write_to(f, image::ImageFormat::Png).unwrap();
        let img = OCR.recognize(image)?;
        println!(
            "  text: {}",
            img.first().map(|x| x.text.clone()).unwrap_or_default()
        );
    }
    Ok(())
}

static OCR: LazyLock<OcrEngine> = LazyLock::new(|| {
    OcrEngine::from_bytes(
        include_bytes!("../models/PP-OCRv5_mobile_det.mnn"),
        include_bytes!("../models/PP-OCRv5_mobile_rec.mnn"),
        include_bytes!("../models/ppocr_keys_v5.txt"),
        None,
    )
    .unwrap()
});
