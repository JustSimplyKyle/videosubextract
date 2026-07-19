//! Safe Rust boundary for the original VideoSubFinder C++ implementation.

use eyre::{Context, ContextCompat, ensure};
use opencv::{
    core::{CV_8UC1, CV_8UC3, Mat, Scalar},
    prelude::*,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::Mutex,
    time::Duration,
};

use crate::video_player::VideoFrame;

unsafe extern "C" {
    fn vsf_headless_api_version() -> i32;
    #[cfg(test)]
    fn vsf_headless_transform_bgr(
        bgr: *const u8,
        bgr_len: usize,
        width: i32,
        height: i32,
        transformed: *mut u8,
        transformed_len: usize,
    ) -> i32;
    fn vsf_headless_search(
        width: i32,
        height: i32,
        params: *const NativeSearchParamsFfi,
        context: *mut c_void,
        next_frame: unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut i64) -> i32,
        segment: unsafe extern "C" fn(
            *mut c_void,
            i64,
            i64,
            *const u8,
            usize,
            i32,
            i32,
            i32,
        ) -> i32,
    ) -> i32;
}

pub const EXPECTED_API_VERSION: i32 = 2;

pub fn api_version() -> i32 {
    // SAFETY: This function takes no pointers and has no preconditions.
    unsafe { vsf_headless_api_version() }
}

/// Parameters used by the original VideoSubFinder `FastSearchSubtitles`
/// implementation. These defaults match the upstream C++ globals except that
/// the headless integration defaults to four worker threads.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Serialize)]
pub struct NativeSearchParams {
    pub threads: i32,
    pub min_subtitle_frames: i32,
    pub text_percent: f64,
    pub min_text_length: f64,
    pub vertical_edges_line_error: f64,
    pub ila_points_line_error: f64,
    pub max_frame_gap_down: i32,
    pub max_frame_gap_up: i32,
    pub use_isa_images: bool,
    pub use_ila_images: bool,
    pub replace_isa_with_filtered: bool,
    pub apply_ocr_image_cleanup: bool,
}

impl Default for NativeSearchParams {
    fn default() -> Self {
        Self {
            threads: 4,
            min_subtitle_frames: 6,
            text_percent: 0.3,
            min_text_length: 0.022,
            vertical_edges_line_error: 0.3,
            ila_points_line_error: 0.3,
            max_frame_gap_down: 20,
            max_frame_gap_up: 40,
            use_isa_images: true,
            use_ila_images: true,
            replace_isa_with_filtered: true,
            apply_ocr_image_cleanup: true,
        }
    }
}

impl NativeSearchParams {
    pub fn validate(&self) -> eyre::Result<()> {
        ensure!(
            (1..=256).contains(&self.threads),
            "threads must be in 1..=256"
        );
        ensure!(
            (1..=1000).contains(&self.min_subtitle_frames),
            "minimum subtitle frames must be in 1..=1000"
        );
        for (name, value) in [
            ("text percent", self.text_percent),
            ("minimum text length", self.min_text_length),
            ("vertical edges line error", self.vertical_edges_line_error),
            ("ILA points line error", self.ila_points_line_error),
        ] {
            ensure!(
                value.is_finite() && (0.0..=1.0).contains(&value),
                "{name} must be finite and in 0..=1"
            );
        }
        ensure!(
            (0..=1000).contains(&self.max_frame_gap_down)
                && (0..=1000).contains(&self.max_frame_gap_up),
            "frame gap parameters must be in 0..=1000"
        );
        Ok(())
    }
}

#[repr(C)]
struct NativeSearchParamsFfi {
    threads: i32,
    min_subtitle_frames: i32,
    text_percent: f64,
    min_text_length: f64,
    vertical_edges_line_error: f64,
    ila_points_line_error: f64,
    max_frame_gap_down: i32,
    max_frame_gap_up: i32,
    use_isa_images: i32,
    use_ila_images: i32,
    replace_isa_with_filtered: i32,
    apply_ocr_image_cleanup: i32,
}

impl From<&NativeSearchParams> for NativeSearchParamsFfi {
    fn from(params: &NativeSearchParams) -> Self {
        Self {
            threads: params.threads,
            min_subtitle_frames: params.min_subtitle_frames,
            text_percent: params.text_percent,
            min_text_length: params.min_text_length,
            vertical_edges_line_error: params.vertical_edges_line_error,
            ila_points_line_error: params.ila_points_line_error,
            max_frame_gap_down: params.max_frame_gap_down,
            max_frame_gap_up: params.max_frame_gap_up,
            use_isa_images: i32::from(params.use_isa_images),
            use_ila_images: i32::from(params.use_ila_images),
            replace_isa_with_filtered: i32::from(params.replace_isa_with_filtered),
            apply_ocr_image_cleanup: i32::from(params.apply_ocr_image_cleanup),
        }
    }
}

/// Apply VideoSubFinder's native C++ transform to a packed BGR OpenCV frame.
///
/// The returned boolean is the legacy algorithm's candidate-presence result;
/// the returned `CV_8UC1` matrix is its transformed text mask.
#[cfg(test)]
pub fn transform_bgr(frame: &Mat) -> eyre::Result<(bool, Mat)> {
    ensure!(
        frame.typ() == CV_8UC3,
        "VideoSubFinder expects CV_8UC3 BGR input"
    );

    let width = frame.cols();
    let height = frame.rows();
    ensure!(
        width > 0 && height > 0,
        "VideoSubFinder received an empty frame"
    );

    // ROIs can have a wider stride than their visible data. Make a packed copy
    // before crossing the C ABI so C++ receives exactly width * height * 3 bytes.
    let mut packed = Mat::default();
    frame
        .copy_to(&mut packed)
        .context("packing the VideoSubFinder input frame")?;
    let input = packed
        .data_bytes()
        .context("reading the VideoSubFinder input frame")?;

    let mut transformed =
        Mat::new_rows_cols_with_default(height, width, CV_8UC1, Scalar::all(0.0))?;
    let output = transformed
        .data_bytes_mut()
        .context("allocating the VideoSubFinder output mask")?;

    // SAFETY: Both slices remain alive and exclusively borrowed for the call;
    // their exact lengths are supplied and validated again by the C++ bridge.
    let status = unsafe {
        vsf_headless_transform_bgr(
            input.as_ptr(),
            input.len(),
            width,
            height,
            output.as_mut_ptr(),
            output.len(),
        )
    };
    ensure!(
        status >= 0,
        "VideoSubFinder C++ transform failed with status {status}"
    );

    Ok((status != 0, transformed))
}

pub struct NativeSubtitleEvent {
    pub start_timestamp: Duration,
    pub end_timestamp: Duration,
    pub ocr_image: Mat,
}

struct FrameSource<I> {
    first: Option<VideoFrame>,
    remaining: I,
}

struct SearchContext<I, F> {
    frames: Mutex<FrameSource<I>>,
    callback: Mutex<F>,
    error: Mutex<Option<String>>,
    width: i32,
    height: i32,
}

impl<I, F> SearchContext<I, F> {
    fn record_error(&self, error: impl Into<String>) {
        let mut stored = self.error.lock().unwrap_or_else(|error| error.into_inner());
        if stored.is_none() {
            *stored = Some(error.into());
        }
    }
}

unsafe extern "C" fn next_frame<I, F>(
    context: *mut c_void,
    destination: *mut u8,
    destination_len: usize,
    timestamp_ms: *mut i64,
) -> i32
where
    I: Iterator<Item = VideoFrame> + Send,
    F: FnMut(NativeSubtitleEvent) -> eyre::Result<()> + Send,
{
    // SAFETY: `find_subtitles` keeps this context alive until the synchronous
    // C++ search and all of its worker tasks have returned.
    let context = unsafe { &*context.cast::<SearchContext<I, F>>() };
    let result = catch_unwind(AssertUnwindSafe(|| -> eyre::Result<i32> {
        let mut source = context
            .frames
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(frame) = source.first.take().or_else(|| source.remaining.next()) else {
            return Ok(0);
        };

        ensure!(
            frame.mat.typ() == CV_8UC3
                && frame.mat.cols() == context.width
                && frame.mat.rows() == context.height,
            "VideoSubFinder frame dimensions or pixel format changed during search"
        );

        let mut packed = Mat::default();
        frame.mat.copy_to(&mut packed)?;
        let input = packed.data_bytes()?;
        ensure!(
            input.len() == destination_len,
            "VideoSubFinder frame buffer size mismatch"
        );
        ensure!(
            !destination.is_null() && !timestamp_ms.is_null(),
            "null native frame output"
        );

        // SAFETY: C++ supplies a writable allocation of `destination_len`, and
        // the exact length was checked against the packed Mat above.
        unsafe {
            ptr::copy_nonoverlapping(input.as_ptr(), destination, input.len());
            *timestamp_ms = i64::try_from(frame.timestamp.as_millis())
                .context("video timestamp exceeds i64 milliseconds")?;
        }
        Ok(1)
    }));

    match result {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            context.record_error(error.to_string());
            -1
        }
        Err(_) => {
            context.record_error("panic in VideoSubFinder frame callback");
            -1
        }
    }
}

unsafe extern "C" fn detected_segment<I, F>(
    context: *mut c_void,
    start_ms: i64,
    end_ms: i64,
    image: *const u8,
    image_len: usize,
    width: i32,
    height: i32,
    channels: i32,
) -> i32
where
    I: Iterator<Item = VideoFrame> + Send,
    F: FnMut(NativeSubtitleEvent) -> eyre::Result<()> + Send,
{
    // SAFETY: See `next_frame`; result callbacks complete before the native
    // search returns and releases the stack-owned context.
    let context = unsafe { &*context.cast::<SearchContext<I, F>>() };
    let result = catch_unwind(AssertUnwindSafe(|| -> eyre::Result<()> {
        ensure!(
            start_ms >= 0 && end_ms >= start_ms,
            "invalid native subtitle timestamps"
        );
        ensure!(
            width > 0 && height > 0 && !image.is_null(),
            "invalid native OCR image"
        );
        ensure!(
            matches!(channels, 1 | 3),
            "invalid native OCR image channels"
        );
        let expected_len = usize::try_from(width)?
            .checked_mul(usize::try_from(height)?)
            .and_then(|pixels| pixels.checked_mul(channels as usize))
            .context("native OCR image is too large")?;
        ensure!(
            image_len == expected_len,
            "native OCR image buffer size mismatch"
        );

        let image_type = if channels == 1 { CV_8UC1 } else { CV_8UC3 };
        let mut ocr_image =
            Mat::new_rows_cols_with_default(height, width, image_type, Scalar::all(255.0))?;
        let output = ocr_image.data_bytes_mut()?;
        // SAFETY: C++ guarantees the image remains valid for this callback and
        // supplies the validated byte length.
        let input = unsafe { std::slice::from_raw_parts(image, image_len) };
        output.copy_from_slice(input);

        context
            .callback
            .lock()
            .unwrap_or_else(|error| error.into_inner())(NativeSubtitleEvent {
            start_timestamp: Duration::from_millis(start_ms as u64),
            end_timestamp: Duration::from_millis(end_ms as u64),
            ocr_image,
        })
    }));

    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            context.record_error(error.to_string());
            -1
        }
        Err(_) => {
            context.record_error("panic in VideoSubFinder segment callback");
            -1
        }
    }
}

/// Run VideoSubFinder's native temporal detector over decoded BGR frames,
/// delivering either `FindTextLines` output or the temporal BGR frame according
/// to `params`, in timestamp order as detections are found.
pub fn find_subtitles_with<I, F>(
    mut frames: I,
    params: &NativeSearchParams,
    callback: F,
) -> eyre::Result<()>
where
    I: Iterator<Item = VideoFrame> + Send,
    F: FnMut(NativeSubtitleEvent) -> eyre::Result<()> + Send,
{
    params.validate()?;
    let Some(first) = frames.next() else {
        return Ok(());
    };
    ensure!(
        first.mat.typ() == CV_8UC3,
        "VideoSubFinder expects CV_8UC3 BGR input"
    );
    let width = first.mat.cols();
    let height = first.mat.rows();
    ensure!(
        width > 0 && height > 0,
        "VideoSubFinder received an empty frame"
    );

    let context = SearchContext {
        frames: Mutex::new(FrameSource {
            first: Some(first),
            remaining: frames,
        }),
        callback: Mutex::new(callback),
        error: Mutex::new(None),
        width,
        height,
    };

    // SAFETY: The callbacks are monomorphized for `I`, the context has the
    // matching type, and the C++ function is synchronous through worker joins.
    let ffi_params = NativeSearchParamsFfi::from(params);
    let status = unsafe {
        vsf_headless_search(
            width,
            height,
            ptr::from_ref(&ffi_params),
            ptr::from_ref(&context).cast_mut().cast(),
            next_frame::<I, F>,
            detected_segment::<I, F>,
        )
    };

    if let Some(error) = context
        .error
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        return Err(eyre::eyre!(error));
    }
    ensure!(
        status >= 0,
        "VideoSubFinder native search failed with status {status}"
    );

    Ok(())
}

#[cfg(test)]
pub fn find_subtitles<I>(frames: I) -> eyre::Result<Vec<NativeSubtitleEvent>>
where
    I: Iterator<Item = VideoFrame> + Send,
{
    let mut segments = Vec::new();
    find_subtitles_with(frames, &NativeSearchParams::default(), |segment| {
        segments.push(segment);
        Ok(())
    })?;
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencv::{core::Point, imgproc};

    #[test]
    fn links_the_native_cpp_backend() {
        assert_eq!(api_version(), EXPECTED_API_VERSION);
    }

    #[test]
    fn transforms_a_frame_through_cpp() {
        let frame = Mat::new_rows_cols_with_default(160, 640, CV_8UC3, Scalar::all(0.0)).unwrap();

        let (has_candidate, mask) = transform_bgr(&frame).unwrap();

        assert!(!has_candidate);
        assert_eq!(mask.rows(), frame.rows());
        assert_eq!(mask.cols(), frame.cols());
        assert_eq!(mask.typ(), CV_8UC1);
    }

    #[test]
    fn searches_a_frame_stream_through_cpp() {
        let frames = (0..20).map(|index| VideoFrame {
            mat: Mat::new_rows_cols_with_default(160, 640, CV_8UC3, Scalar::all(0.0)).unwrap(),
            timestamp: Duration::from_millis(index * 40),
        });

        let events = find_subtitles(frames).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn detects_and_cleans_a_synthetic_subtitle() {
        let frames = (0..45).map(|index| {
            let mut mat =
                Mat::new_rows_cols_with_default(160, 640, CV_8UC3, Scalar::all(0.0)).unwrap();
            if (8..32).contains(&index) {
                imgproc::put_text(
                    &mut mat,
                    "SUBTITLE TEST",
                    Point::new(110, 105),
                    imgproc::FONT_HERSHEY_SIMPLEX,
                    1.2,
                    Scalar::all(255.0),
                    3,
                    imgproc::LINE_AA,
                    false,
                )
                .unwrap();
            }
            VideoFrame {
                mat,
                timestamp: Duration::from_millis(index * 40),
            }
        });

        let events = find_subtitles(frames).unwrap();
        assert!(!events.is_empty());
        assert!(events.iter().all(|event| event.ocr_image.typ() == CV_8UC1));
        assert!(events.iter().any(|event| event.ocr_image.cols() == 640 * 4));
    }

    #[test]
    fn can_skip_native_ocr_image_cleanup() {
        let frames = (0..45).map(|index| {
            let mut mat =
                Mat::new_rows_cols_with_default(160, 640, CV_8UC3, Scalar::all(0.0)).unwrap();
            if (8..32).contains(&index) {
                imgproc::put_text(
                    &mut mat,
                    "SUBTITLE TEST",
                    Point::new(110, 105),
                    imgproc::FONT_HERSHEY_SIMPLEX,
                    1.2,
                    Scalar::all(255.0),
                    3,
                    imgproc::LINE_AA,
                    false,
                )
                .unwrap();
            }
            VideoFrame {
                mat,
                timestamp: Duration::from_millis(index * 40),
            }
        });
        let mut params = NativeSearchParams::default();
        params.apply_ocr_image_cleanup = false;
        let mut events = Vec::new();

        find_subtitles_with(frames, &params, |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();

        assert!(!events.is_empty());
        assert!(events.iter().all(|event| event.ocr_image.typ() == CV_8UC3));
        assert!(events.iter().all(|event| event.ocr_image.cols() == 640));
    }
}
