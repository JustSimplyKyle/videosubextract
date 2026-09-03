use crate::config::{ProcessingResolution, SubtitleDetector};
use crate::native_video_sub_finder::{
    NativeSearchParams, NativeSubtitleEvent, find_subtitles_with,
};
use crate::ocr::{OcrModel, OcrProvider};
use crate::subfinder::{Params as RustSearchParams, SubtitleSearch};
use crate::video_player::{CropRect, VideoFrame, create_video_player, mat_to_rgba};
use eyre::Context;
use futures::{Stream, StreamExt};
use image::{DynamicImage, RgbaImage};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

const OCR_PARALLELISM: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subtitle {
    pub start_timestamp: Duration,
    pub end_timestamp: Duration,
    pub text: String,
}

pub fn to_srt(results: &[Subtitle]) -> String {
    use std::fmt::Write;

    results
        .iter()
        .enumerate()
        .fold(String::new(), |mut out, (i, result)| {
            let start = srt_timestamp(result.start_timestamp);
            let end = srt_timestamp(result.end_timestamp);
            writeln!(
                out,
                "{}\n{} --> {}\n{}\n",
                i + 1,
                start,
                end,
                result.text.trim()
            )
            .ok();
            out
        })
}

fn srt_timestamp(timestamp: Duration) -> String {
    let total_ms = timestamp.as_millis();
    let ms = total_ms % 1000;
    let seconds = (total_ms / 1000) % 60;
    let minutes = (total_ms / 60_000) % 60;
    let hours = total_ms / 3_600_000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{ms:03}")
}

#[derive(Clone)]
pub struct OcrHandle(Arc<RwLock<OcrModel>>);

impl OcrHandle {
    pub fn new(model: OcrModel) -> Self {
        Self(Arc::new(RwLock::new(model)))
    }

    fn read(&self) -> OcrModel {
        self.0
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn set(&self, model: OcrModel) {
        *self.0.write().unwrap_or_else(|error| error.into_inner()) = model;
    }
}

#[derive(Clone)]
pub struct Request {
    pub input: PathBuf,
    pub crop: Option<CropRect>,
    pub ocr: OcrHandle,
    pub detector: SubtitleDetector,
    pub native_search_params: NativeSearchParams,
    pub post_ocr_processing: bool,
    pub processing_resolution: ProcessingResolution,
    pub progress_interval: usize,
    pub include_progress_preview: bool,
}

#[derive(Debug)]
pub enum Event {
    Progress {
        timestamp: Duration,
        preview: Option<RgbaImage>,
    },
    SubtitleFound {
        subtitle: Subtitle,
        preview: RgbaImage,
        replace_previous: bool,
    },
    Finished,
    Error(String),
}

pub fn stream(request: Request) -> impl Stream<Item = Event> + Send {
    let (event_tx, event_rx) = async_channel::bounded(OCR_PARALLELISM);
    smol::spawn(run(request, event_tx)).detach();

    futures::stream::unfold(event_rx, |receiver| async move {
        receiver.recv().await.ok().map(|event| (event, receiver))
    })
}

async fn run(request: Request, event_tx: async_channel::Sender<Event>) {
    let (subtitle_tx, subtitle_rx) =
        async_channel::bounded::<NativeSubtitleEvent>(OCR_PARALLELISM);
    let (completion_tx, completion_rx) = futures::channel::oneshot::channel();
    let blocking_event_tx = event_tx.clone();
    let input = request.input;
    let crop = request.crop;
    let detector = request.detector;
    let native_search_params = request.native_search_params;
    let processing_resolution = request.processing_resolution;
    let progress_interval = request.progress_interval.max(1);
    let include_progress_preview = request.include_progress_preview;

    smol::spawn(smol::unblock(move || {
        let result = (|| {
            let input = ffmpeg_the_third::format::input(&input)
                .wrap_err("opening the video with FFmpeg")?;
            let (controller, iter) =
                create_video_player::<false>(input, crop, processing_resolution)
                    .wrap_err("initializing the video decoder")?;

            let frame_iter = ProgressIter {
                inner: iter.filter_map(Result::ok),
                event_tx: blocking_event_tx,
                count: 0,
                interval: progress_interval,
                include_preview: include_progress_preview,
            };

            match detector {
                SubtitleDetector::OriginalCpp => {
                    find_subtitles_with(frame_iter, &native_search_params, |event| {
                        subtitle_tx
                            .send_blocking(event)
                            .map_err(|_| eyre::eyre!("subtitle OCR receiver closed"))
                    })
                }
                SubtitleDetector::RustRewrite => {
                    for event in SubtitleSearch::new(frame_iter, RustSearchParams::default()) {
                        let event = NativeSubtitleEvent {
                            start_timestamp: event.start_timestamp,
                            end_timestamp: event.end_timestamp,
                            ocr_image: event.sample_bgr,
                        };
                        subtitle_tx
                            .send_blocking(event)
                            .map_err(|_| eyre::eyre!("subtitle OCR receiver closed"))?;
                    }
                    Ok(())
                }
            }
        })();

        completion_tx.send(result).ok();
    }))
    .detach();

    let ocr = request.ocr;
    let jobs = futures::stream::unfold(subtitle_rx, |receiver| async move {
        receiver.recv().await.ok().map(|event| (event, receiver))
    })
    .map(move |event| {
        let ocr = ocr.clone();
        smol::unblock(move || {
            let preview = mat_to_rgba(&event.ocr_image)?;
            let image = DynamicImage::ImageRgba8(preview.clone());
            let text = ocr
                .read()
                .recognize_text(&image)
                .wrap_err("recognizing subtitle text")?;

            eyre::Ok((
                Subtitle {
                    start_timestamp: event.start_timestamp,
                    end_timestamp: event.end_timestamp,
                    text,
                },
                preview,
            ))
        })
    });

    let mut jobs = Box::pin(jobs.buffered(OCR_PARALLELISM));
    let mut previous: Option<Subtitle> = None;

    while let Some(result) = jobs.next().await {
        let (mut subtitle, preview) = match result {
            Ok(result) => result,
            Err(error) => {
                event_tx.send(Event::Error(format!("{error:#}"))).await.ok();
                return;
            }
        };

        if subtitle.text.trim().is_empty() {
            continue;
        }

        let replace_previous = previous.as_ref().is_some_and(|previous| {
            should_replace_previous(previous, &subtitle, request.post_ocr_processing)
        });

        if replace_previous && let Some(previous) = &previous {
            subtitle.start_timestamp = previous.start_timestamp;
        }
        previous = Some(subtitle.clone());

        if event_tx
            .send(Event::SubtitleFound {
                subtitle,
                preview,
                replace_previous,
            })
            .await
            .is_err()
        {
            return;
        }
    }

    match completion_rx.await {
        Ok(Ok(())) => {
            event_tx.send(Event::Finished).await.ok();
        }
        Ok(Err(error)) => {
            event_tx.send(Event::Error(format!("{error:#}"))).await.ok();
        }
        Err(_) => {
            event_tx
                .send(Event::Error(
                    "subtitle detector stopped unexpectedly".into(),
                ))
                .await
                .ok();
        }
    }
}

fn should_replace_previous(previous: &Subtitle, current: &Subtitle, enabled: bool) -> bool {
    enabled
        && current
            .start_timestamp
            .saturating_sub(previous.end_timestamp)
            < Duration::from_secs(5)
        && previous.text.trim() == current.text.trim()
}

struct ProgressIter<I> {
    inner: I,
    event_tx: async_channel::Sender<Event>,
    count: usize,
    interval: usize,
    include_preview: bool,
}

impl<I: Iterator<Item = VideoFrame>> Iterator for ProgressIter<I> {
    type Item = VideoFrame;

    fn next(&mut self) -> Option<Self::Item> {
        let frame = self.inner.next()?;
        self.count += 1;

        if self.count.is_multiple_of(self.interval) {
            let preview = self
                .include_preview
                .then(|| mat_to_rgba(&frame.mat).ok())
                .flatten();
            self.event_tx
                .send_blocking(Event::Progress {
                    timestamp: frame.timestamp,
                    preview,
                })
                .ok();
        }

        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_srt_timestamps() {
        let subtitles = [Subtitle {
            start_timestamp: Duration::from_millis(1_234),
            end_timestamp: Duration::from_millis(65_678),
            text: " Hello ".into(),
        }];

        assert_eq!(
            to_srt(&subtitles),
            "1\n00:00:01,234 --> 00:01:05,678\nHello\n\n"
        );
    }

    #[test]
    fn adjacent_duplicate_subtitles_are_replaced_when_enabled() {
        let previous = Subtitle {
            start_timestamp: Duration::ZERO,
            end_timestamp: Duration::from_secs(1),
            text: "same text".into(),
        };
        let current = Subtitle {
            start_timestamp: Duration::from_secs(2),
            end_timestamp: Duration::from_secs(3),
            text: " same text ".into(),
        };

        assert!(should_replace_previous(&previous, &current, true));
        assert!(!should_replace_previous(&previous, &current, false));
    }
}
