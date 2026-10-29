use crate::config::ProcessingResolution;
use eyre::{Context, ContextCompat};
use ffmpeg_the_third::{
    self as ffmpeg, Rational, Rescale, codec,
    ffi::AV_TIME_BASE,
    filter::Graph,
    format::Pixel,
    frame::Video,
    media::{self},
    rescale::TIME_BASE,
    threading,
};
use opencv::{
    core::{CV_8UC3, Scalar},
    prelude::*,
};

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) struct PlayerState {
    pub input: ffmpeg::format::context::Input,
    pub decoder: codec::decoder::Video,
    pub filter_graph: Option<GraphWithInfo>,
    pub frame_buffer: VecDeque<eyre::Result<VideoFrame>>,
    pub timestamp: Option<Duration>,
    pub seek_generation: usize,
}

pub(crate) struct InnerPlayer {
    pub(crate) state: Mutex<PlayerState>,
    pub(crate) stream_index: usize,
    pub info: DecoderInfo,
}

#[derive(Clone)]
pub struct VideoPlayerController {
    pub(crate) inner: Arc<InnerPlayer>,
}

impl std::hash::Hash for VideoPlayerController {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.inner) as usize).hash(state);
    }
}

#[derive(Clone)]
pub struct VideoPlayerIterator<const STOP_ON_SEEK: bool> {
    pub(crate) inner: Arc<InnerPlayer>,
    pub(crate) current_generation: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// `sample_interval` drops frames closer than that to the last one kept, so
/// everything after the decoder only sees about one frame per interval.
pub fn create_video_player<const STOP_ON_SEEK: bool>(
    input: ffmpeg::format::context::Input,
    crop_rectangle: impl Into<Option<CropRect>>,
    processing_resolution: ProcessingResolution,
    sample_interval: Option<Duration>,
) -> eyre::Result<(VideoPlayerController, VideoPlayerIterator<STOP_ON_SEEK>)> {
    let crop_rectangle = crop_rectangle.into();
    let vstream = input
        .streams()
        .best(media::Type::Video)
        .context("Video stream not found")?;

    let stream_index = vstream.index();
    let avg_frame_rate = f64::from(vstream.avg_frame_rate());

    let video_time = {
        let duration = vstream.duration().max(input.duration());
        let duration_sec = duration as f64 / AV_TIME_BASE as f64;
        Duration::from_secs_f64(duration_sec)
    };

    let mut vcodec = codec::context::Context::from_parameters(vstream.parameters())?;
    if let Ok(parallelism) = std::thread::available_parallelism() {
        vcodec.set_threading(threading::Config {
            kind: threading::Type::Frame,
            count: parallelism.get().max(16),
        });
    }

    let mut decoder = vcodec.decoder().video()?;
    if sample_interval.is_some() {
        // B-frames are never references for the I/P frames, and those alone are
        // still far denser than any useful sample rate. Skipping them saves most
        // of the decoding work.
        decoder.skip_frame(codec::discard::Discard::Bidirectional);
    }

    let time_base = vstream.time_base();

    let info = DecoderInfo {
        format: decoder.format(),
        width: decoder.width(),
        height: decoder.height(),
        frame_rate: avg_frame_rate,
        video_time,
        time_base,
    };

    let filter_graph = Some(build_filter_graph(
        info,
        crop_rectangle,
        processing_resolution,
        sample_interval,
    )?);

    let inner = Arc::new(InnerPlayer {
        state: Mutex::new(PlayerState {
            input,
            decoder,
            filter_graph,
            frame_buffer: Default::default(),
            seek_generation: 0,
            timestamp: None,
        }),
        stream_index,
        info,
    });

    Ok((
        VideoPlayerController {
            inner: inner.clone(),
        },
        VideoPlayerIterator {
            inner,
            current_generation: 0,
        },
    ))
}

#[derive(Copy, Clone, Debug)]
enum Direction {
    Forward,
    Backward,
    Absolute,
}

impl VideoPlayerController {
    pub fn info(&self) -> &DecoderInfo {
        &self.inner.info
    }

    /// A new frame iterator sharing this controller's decoder.
    pub fn frames<const STOP_ON_SEEK: bool>(&self) -> VideoPlayerIterator<STOP_ON_SEEK> {
        VideoPlayerIterator {
            inner: self.inner.clone(),
            current_generation: 0,
        }
    }

    fn seek(&self, delta: Duration, direction: Direction) -> eyre::Result<()> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| eyre::eyre!("lock poisoned"))?;

        let current_secs = state.timestamp.map(|x| x.as_secs_f64()).unwrap_or_default();

        let target_secs = match direction {
            Direction::Forward => current_secs + delta.as_secs_f64(),
            Direction::Backward => (current_secs - delta.as_secs_f64()).max(0.),
            Direction::Absolute => delta.as_secs_f64(),
        };

        let target_us = (target_secs * f64::from(AV_TIME_BASE)) as i64;

        state.input.seek(target_us, i64::MIN..=target_us)?;
        state.decoder.flush();
        state.frame_buffer.clear();

        state.seek_generation += 1;

        drop(state);
        Ok(())
    }
    pub fn seek_forward(&self, delta: Duration) -> eyre::Result<()> {
        self.seek(delta, Direction::Forward)
    }
    pub fn seek_backward(&self, delta: Duration) -> eyre::Result<()> {
        self.seek(delta, Direction::Backward)
    }
    pub fn seek_absolute(&self, timestamp: Duration) -> eyre::Result<()> {
        self.seek(timestamp, Direction::Absolute)
    }
}

pub struct VideoFrame {
    pub mat: Mat,
    pub timestamp: Duration,
}

pub fn mat_to_rgba(mat: &opencv::core::Mat) -> eyre::Result<image::RgbaImage> {
    use opencv::core::{MatTraitConst, MatTraitConstManual};

    let rows = u32::try_from(mat.rows()).context("image has a negative height")?;
    let cols = u32::try_from(mat.cols()).context("image has a negative width")?;
    let channels = mat.channels();

    // A Mat ROI can have a stride wider than its visible rows. `copy_to` makes
    // an independent, packed copy; `try_clone` would only clone the Mat header.
    let mut packed = opencv::core::Mat::default();
    mat.copy_to(&mut packed)
        .context("failed to copy image into packed storage")?;
    let pixels = packed
        .data_bytes()
        .context("failed to access packed image bytes")?;

    let pixel_count = (cols as usize)
        .checked_mul(rows as usize)
        .ok_or_else(|| eyre::eyre!("image dimensions are too large"))?;
    let expected_len = pixel_count
        .checked_mul(channels as usize)
        .ok_or_else(|| eyre::eyre!("image buffer is too large"))?;
    if !matches!(channels, 1 | 3 | 4) || pixels.len() != expected_len {
        return Err(eyre::eyre!(
            "expected a packed 8-bit grayscale, BGR, or BGRA Mat; got {channels} channels and {} bytes for a {cols}x{rows} image",
            pixels.len()
        ));
    }

    let mut rgba = Vec::with_capacity(pixel_count * 4);
    match channels {
        1 => rgba.extend(pixels.iter().flat_map(|&v| [v, v, v, 255])),
        3 => rgba.extend(pixels.chunks_exact(3).flat_map(|p| [p[2], p[1], p[0], 255])),
        4 => rgba.extend(
            pixels
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], p[3]]),
        ),
        _ => unreachable!("channel count was checked above"),
    }

    image::RgbaImage::from_raw(cols, rows, rgba)
        .ok_or_else(|| eyre::eyre!("failed to construct RGBA image"))
}

impl<const STOP_ON_SEEK: bool> Iterator for VideoPlayerIterator<STOP_ON_SEEK> {
    type Item = eyre::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut state = self.inner.state.lock().expect("state lock poisoned");

        // If a seek happened via the controller, this iterator's generation
        // is out of date. Return None immediately to close out if `StopOnSeek` is set to true
        if STOP_ON_SEEK && state.seek_generation != self.current_generation {
            return None;
        }

        // 1. If we have frames from a previous packet, yield one immediately!
        if let Some(mat) = state.frame_buffer.pop_front() {
            state.timestamp = mat.as_ref().ok().map(|x| x.timestamp);
            return Some(mat);
        }

        let info = self.inner.info;

        // 2. Pull packets until we generate at least one frame
        while let Some(Ok((stream, packet))) = state.input.packets().next() {
            // Ignore audio/subtitle packets
            if stream.index() != self.inner.stream_index {
                continue;
            }

            // Send to decoder
            if state.decoder.send_packet(&packet).is_err() {
                continue;
            }

            // Receive all available frames for this packet
            let mut decoded_video = Video::empty();
            while state.decoder.receive_frame(&mut decoded_video).is_ok() {
                frame_to_mats(&mut state, &info, &decoded_video);
            }

            // If this packet generated frames, yield the first one,
            // the rest stay in the buffer for the next calls!
            if let Some(mat) = state.frame_buffer.pop_front() {
                state.timestamp = mat.as_ref().ok().map(|x| x.timestamp);
                return Some(mat);
            }
        }

        // 3. EOF: Flush the decoder when packets run out
        state.decoder.send_eof().ok();
        let mut decoded_video = Video::empty();
        while state.decoder.receive_frame(&mut decoded_video).is_ok() {
            frame_to_mats(&mut state, &info, &decoded_video);
        }

        state.frame_buffer.pop_front()
    }
}

pub(crate) fn frame_to_mats(
    state: &mut std::sync::MutexGuard<'_, PlayerState>,
    info: &DecoderInfo,
    decoded_video: &Video,
) {
    let frames = match state.filter_graph.as_mut().unwrap().apply(decoded_video) {
        Ok(frames) => frames,
        Err(e) => {
            state.frame_buffer.push_back(Err(e));
            return;
        }
    };

    for frame in frames {
        let timestamp = frame.timestamp().expect("frame has no timestamp");
        let us = timestamp.rescale(info.time_base, TIME_BASE);
        let timestamp = Duration::from_micros(us as u64);

        let mat = video_frame_to_mat(&frame, frame.width() as i32, frame.height() as i32);
        let mat = mat.map(|mat| VideoFrame { mat, timestamp });
        state.frame_buffer.push_back(mat);
    }
}

#[derive(Clone, Copy)]
pub struct DecoderInfo {
    pub(crate) format: Pixel,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub frame_rate: f64,
    pub video_time: Duration,
    pub time_base: Rational,
}

// FFmpeg names an instance written as `filter@id` in a spec by that whole string.
const GRAPH_SOURCE: &str = "buffer@in";
const GRAPH_SINK: &str = "buffersink@out";

pub(crate) fn build_filter_graph(
    info: DecoderInfo,
    crop_rectangle: Option<CropRect>,
    processing_resolution: ProcessingResolution,
    sample_interval: Option<Duration>,
) -> eyre::Result<GraphWithInfo> {
    let pix_fmt_name = info
        .format
        .descriptor()
        .context("pixel format has no descriptor")?
        .name();

    let (crop_w, crop_h) = crop_rectangle.map_or((info.width, info.height), |rect| {
        (rect.width as u32, rect.height as u32)
    });
    let scaled = scaled_output_dimensions(info, crop_w, crop_h, processing_resolution);

    // `select` keeps each frame's original pts, unlike `fps` (or `-r`), which
    // retimes frames onto its own clock and duplicates them to fill gaps.
    let desc = [
        Some(format!(
            "{GRAPH_SOURCE}=video_size={}x{}:pix_fmt={pix_fmt_name}:time_base={}/{}:sar=1/1",
            info.width,
            info.height,
            info.time_base.numerator(),
            info.time_base.denominator(),
        )),
        sample_interval.map(|interval| {
            format!(
                "select='isnan(prev_selected_t)+gte(t-prev_selected_t\\,{})'",
                interval.as_secs_f64()
            )
        }),
        crop_rectangle.map(|rect| {
            format!(
                "crop=x={}:y={}:w={crop_w}:h={crop_h}",
                rect.x as u32, rect.y as u32
            )
        }),
        scaled.map(|(width, height)| format!("scale=w={width}:h={height}:flags=bilinear")),
        Some("format=pix_fmts=bgr24".into()),
        Some(GRAPH_SINK.into()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(",");

    let mut graph = Graph::new();
    graph
        .parse(&desc)
        .with_context(|| format!("parsing filter graph {desc:?}"))?;
    graph.validate()?;

    Ok(GraphWithInfo { graph })
}

fn scaled_output_dimensions(
    info: DecoderInfo,
    output_width: u32,
    output_height: u32,
    processing_resolution: ProcessingResolution,
) -> Option<(u32, u32)> {
    let max_height = processing_resolution.max_height()?;
    if info.height <= max_height {
        return None;
    }

    let scale = f64::from(max_height) / f64::from(info.height);
    Some((
        (f64::from(output_width) * scale).round().max(1.0) as u32,
        (f64::from(output_height) * scale).round().max(1.0) as u32,
    ))
}

pub(crate) struct GraphWithInfo {
    pub(crate) graph: Graph,
}

impl GraphWithInfo {
    /// Push one decoded frame through and collect everything that comes out:
    /// none if the graph dropped it, possibly several from buffered filters.
    pub fn apply(&mut self, frame: &Video) -> eyre::Result<Vec<Video>> {
        self.graph
            .get(GRAPH_SOURCE)
            .context("buffer filter not found")?
            .source()
            .add(frame)
            .context("pushing frame into filter graph (check frame has a valid pts)")?;

        let mut sink_context = self
            .graph
            .get(GRAPH_SINK)
            .context("buffersink filter not found")?;
        let mut sink = sink_context.sink();
        let mut frames = Vec::new();
        loop {
            let mut output = Video::empty();
            match sink.frame(&mut output) {
                Ok(()) => frames.push(output),
                // EAGAIN: the graph needs more input before it can output again.
                Err(ffmpeg::Error::Other { errno })
                    if std::io::Error::from_raw_os_error(errno).kind()
                        == std::io::ErrorKind::WouldBlock =>
                {
                    return Ok(frames);
                }
                Err(ffmpeg::Error::Eof) => return Ok(frames),
                Err(error) => {
                    return Err(error).context("pulling frame out of filter graph");
                }
            }
        }
    }
}

/// Copy a BGR24 ffmpeg frame into an owned `Mat`. A copy (not a zero-copy wrap) is
/// necessary because `bgr_frame` is dropped at the end of each loop iteration above;
/// it's done row-by-row since ffmpeg's `linesize`/stride is often wider than
/// `width * 3` (alignment padding), so it can't be treated as one contiguous slice.
pub(crate) fn video_frame_to_mat(frame: &Video, width: i32, height: i32) -> eyre::Result<Mat> {
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
