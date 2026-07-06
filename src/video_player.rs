use cosmic::iced;
use eyre::{Context, ContextCompat};
use ffmpeg_the_third::{
    self as ffmpeg, Packet, Rescale, codec,
    ffi::{AV_TIME_BASE, av_rescale_q},
    filter::Graph,
    format::Pixel,
    frame::Video,
    media::{self},
    threading,
};
use opencv::{
    core::{CV_8UC3, Scalar},
    prelude::*,
};

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, atomic::AtomicUsize},
    time::Duration,
};

pub(crate) struct PlayerState {
    pub(crate) input: ffmpeg::format::context::Input,
    pub(crate) decoder: codec::decoder::Video,
    pub(crate) filter_graph: Option<GraphWithInfo>,
    pub(crate) frame_buffer: VecDeque<eyre::Result<Mat>>,
    pub(crate) seek_generation: usize,
}

pub(crate) struct InnerPlayer {
    pub(crate) state: Mutex<PlayerState>,
    pub(crate) stream_index: usize,
    pub info: DecoderInfo,
    // We leave this outside the Mutex so a UI thread can poll
    // the progress bar without blocking the decoder.
    pub(crate) current_frame: AtomicUsize,
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

pub struct VideoPlayerIterator<const STOP_ON_SEEK: bool> {
    pub(crate) inner: Arc<InnerPlayer>,
    pub(crate) current_generation: usize,
}

pub fn create_video_player<const STOP_ON_SEEK: bool>(
    input: ffmpeg::format::context::Input,
    crop_rectangle: Option<iced::Rectangle>,
) -> eyre::Result<(VideoPlayerController, VideoPlayerIterator<STOP_ON_SEEK>)> {
    let vstream = input
        .streams()
        .best(media::Type::Video)
        .context("Video stream not found")?;

    let stream_index = vstream.index();
    let avg_frame_rate = f64::from(vstream.avg_frame_rate());

    let mut vcodec = codec::context::Context::from_parameters(vstream.parameters())?;
    if let Ok(parallelism) = std::thread::available_parallelism() {
        vcodec.set_threading(threading::Config {
            kind: threading::Type::Frame,
            count: parallelism.get(),
        });
    }

    let decoder = vcodec.decoder().video()?;

    let info = DecoderInfo {
        format: decoder.format(),
        width: decoder.width(),
        height: decoder.height(),
        frame_rate: avg_frame_rate,
    };

    let filter_graph = if let Some(rect) = crop_rectangle {
        Some(build_crop_graph(
            info,
            rect.x as u32,
            rect.y as u32,
            rect.width as u32,
            rect.height as u32,
        )?)
    } else {
        Some(build_filter_graph(info)?)
    };

    let inner = Arc::new(InnerPlayer {
        state: Mutex::new(PlayerState {
            input,
            decoder,
            filter_graph,
            frame_buffer: Default::default(),
            seek_generation: 0,
        }),
        stream_index,
        info,
        current_frame: 0.into(),
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
}

impl VideoPlayerController {
    fn seek(&self, delta: Duration, direction: Direction) -> eyre::Result<()> {
        println!("attempt seeking {:2}s", delta.as_secs_f64());
        let current_secs = self
            .inner
            .current_frame
            .load(std::sync::atomic::Ordering::Relaxed) as f64
            / self.inner.info.frame_rate;

        let target_secs = match direction {
            Direction::Forward => current_secs + delta.as_secs_f64(),
            Direction::Backward => (current_secs - delta.as_secs_f64()).max(0.),
        };

        let target_us = (target_secs * f64::from(AV_TIME_BASE)) as i64;

        println!("target: {target_us}");

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| eyre::eyre!("lock poisoned"))?;

        println!("got locked");

        state.input.seek(target_us, i64::MIN..=target_us)?;
        state.decoder.flush();

        self.inner.current_frame.store(
            (target_secs * self.inner.info.frame_rate) as usize,
            std::sync::atomic::Ordering::Relaxed,
        );
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
}

impl<const STOP_ON_SEEK: bool> Iterator for VideoPlayerIterator<STOP_ON_SEEK> {
    type Item = eyre::Result<Mat>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut state = self.inner.state.lock().expect("state lock poisoned");

        // If a seek happened via the controller, this iterator's generation
        // is out of date. Return None immediately to close out if `StopOnSeek` is set to true
        if STOP_ON_SEEK && state.seek_generation != self.current_generation {
            return None;
        }

        // 1. If we have frames from a previous packet, yield one immediately!
        if let Some(mat) = state.frame_buffer.pop_front() {
            return Some(mat);
        }

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
                frame_to_mats(&mut state, &decoded_video);
            }

            // If this packet generated frames, yield the first one,
            // the rest stay in the buffer for the next calls!
            if let Some(mat) = state.frame_buffer.pop_front() {
                return Some(mat);
            }
        }

        // 3. EOF: Flush the decoder when packets run out
        let _ = state.decoder.send_eof();
        let mut decoded_video = Video::empty();
        while state.decoder.receive_frame(&mut decoded_video).is_ok() {
            frame_to_mats(&mut state, &decoded_video);
        }

        state.frame_buffer.pop_front()
    }
}

pub(crate) fn frame_to_mats(
    state: &mut std::sync::MutexGuard<'_, PlayerState>,
    decoded_video: &Video,
) {
    let frame = state.filter_graph.as_mut().unwrap().apply(decoded_video);

    let frame = match frame {
        Ok(x) => x,
        Err(e) => {
            state.frame_buffer.push_back(Err(e));
            return;
        }
    };

    let mat = video_frame_to_mat(&frame, frame.width() as i32, frame.height() as i32);
    state.frame_buffer.push_back(mat);
}

#[derive(Clone, Copy)]
pub(crate) struct DecoderInfo {
    pub(crate) format: Pixel,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) frame_rate: f64,
}

pub(crate) fn build_crop_graph(
    info: DecoderInfo,
    crop_x: u32,
    crop_y: u32,
    crop_w: u32,
    crop_h: u32,
) -> eyre::Result<GraphWithInfo> {
    let pix_fmt_name = info
        .format
        .descriptor()
        .context("pixel format has no descriptor")?
        .name();

    let desc = format!(
        "buffer=video_size={w}x{h}:pix_fmt={pix_fmt_name}:time_base=1/1:sar=1/1,\
         crop=x={crop_x}:y={crop_y}:w={crop_w}:h={crop_h},\
         format=pix_fmts=bgr24,\
         buffersink",
        w = info.width,
        h = info.height,
    );

    let mut graph = Graph::new();
    graph.parse(&desc)?;
    graph.validate()?;

    Ok(GraphWithInfo {
        graph,
        source: "Parsed_buffer_0",
        sink: "Parsed_buffersink_3",
    })
}

pub(crate) fn build_filter_graph(info: DecoderInfo) -> eyre::Result<GraphWithInfo> {
    let pix_fmt_name = info
        .format
        .descriptor()
        .context("pixel format has no descriptor")?
        .name();

    let desc = format!(
        "buffer=video_size={w}x{h}:pix_fmt={pix_fmt_name}:time_base=1/1:sar=1/1,\
         format=pix_fmts=bgr24,\
         buffersink",
        w = info.width,
        h = info.height,
    );

    let mut graph = Graph::new();
    graph.parse(&desc)?;
    graph.validate()?;

    Ok(GraphWithInfo {
        graph,
        source: "Parsed_buffer_0",
        sink: "Parsed_buffersink_2",
    })
}

pub(crate) struct GraphWithInfo {
    pub(crate) graph: Graph,
    pub(crate) source: &'static str,
    pub(crate) sink: &'static str,
}

impl GraphWithInfo {
    pub fn apply(&mut self, frame: &Video) -> eyre::Result<Video> {
        self.graph
            .get(self.source)
            .context("buffer filter not found")?
            .source()
            .add(frame)
            .context("pushing frame into filter graph (check frame has a valid pts)")?;

        let mut output = Video::empty();
        self.graph
            .get(self.sink)
            .context("buffersink filter not found")?
            .sink()
            .frame(&mut output)
            .context("pulling frame out of filter graph")?;
        Ok(output)
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
