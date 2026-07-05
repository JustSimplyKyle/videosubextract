//! Rust approximation of VideoSubFinder's "Run Search" + basic OCR-prep cleanup.
//!
//! Per frame: crop to subtitle ROI -> grayscale -> Sobel edges -> threshold -> morph close.
//! Frames are grouped into runs by comparing each new edge mask against the *previous
//! frame's* mask (local comparison — a whole-run running average was tried first, but
//! it tightens as a run grows and antialiasing fringe drops out of the average, which
//! makes IoU decay over time and spuriously splits long-but-unchanged subtitles into
//! fragments too short to pass `min_run_len`, silently dropping them). When a run closes:
//!   - `mask`      = per-pixel "was this an edge in >= stability% of the run's frames"
//!                   (drops transient background edges, keeps stable text strokes)
//!   - `sample_bgr`= per-pixel temporal median of buffered run frames
//!                   (static text stays sharp, moving background gets smoothed out)
//!
//! Still missing vs. the real thing: color-space filtering before edge detection,
//! and any actual OCR. Those are separate stages layered on top of this one.

use opencv::{Result, core, imgproc, prelude::*};

#[derive(Debug, Clone)]
pub struct Params {
    /// Subtitle search region within each frame (e.g. bottom ~20% of the image).
    pub roi: core::Rect,
    /// Sobel-magnitude threshold used to binarize edges.
    pub sobel_thresh: f64,
    /// Below this many "on" pixels in the mask, a frame is treated as "no text".
    pub min_edge_pixels: i32,
    /// IoU between a new frame's mask and the run's running-average mask needed
    /// to say "same subtitle continues".
    pub similarity_thresh: f64,
    /// Minimum consecutive frames for a run to count as a real subtitle.
    pub min_run_len: usize,
    /// Consecutive "no text" frames tolerated inside a run before closing it.
    pub max_gap: usize,
    /// Fraction of a run's frames a pixel must be "edge-on" in to survive into
    /// the final cleaned mask.
    pub mask_stability_thresh: f64,
    /// Cap on how many frames get buffered per run for the median stack;
    /// longer runs are reservoir-subsampled instead of buffering everything.
    pub max_stack_frames: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            roi: core::Rect::new(0, 0, 0, 0), // caller must size this to the actual frame
            sobel_thresh: 60.0,
            min_edge_pixels: 50,
            similarity_thresh: 0.4,
            min_run_len: 5,
            max_gap: 2,
            mask_stability_thresh: 0.6,
            max_stack_frames: 40,
        }
    }
}

#[derive(Debug)]
pub struct SubtitleEvent {
    pub start_frame: usize,
    pub end_frame: usize,
    pub sample_bgr: Mat, // temporal-median frame, cleaner input for OCR
    pub mask: Mat,       // stability-filtered edge mask, reused by clean_for_ocr
}

/// Tiny xorshift64* RNG so reservoir sampling doesn't need an extra crate.
struct SmallRng(u64);
impl SmallRng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E3779B97F4A7C15)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn gen_range(&mut self, upper: usize) -> usize {
        (self.next_u64() % upper as u64) as usize
    }
}

/// Internal state for a subtitle run in progress.
struct Run {
    start_frame: usize,
    len: usize,
    gap: usize,
    mask_accum: Mat, // CV_32F running count of "edge on" per pixel, for the final mask
    last_mask: Mat,  // most recent raw frame mask, used for continuity checks
    bgr_buffer: Vec<Mat>, // reservoir-capped buffer feeding the median stack
    rng: SmallRng,
}

impl Run {
    fn new(start_frame: usize, mask: Mat, bgr: Mat) -> Result<Self> {
        let mut mask_f = Mat::default();
        mask.convert_to(&mut mask_f, core::CV_32F, 1.0 / 255.0, 0.0)?;
        let last_mask = mask.try_clone()?;
        Ok(Self {
            start_frame,
            len: 1,
            gap: 0,
            mask_accum: mask_f,
            last_mask,
            bgr_buffer: vec![bgr],
            rng: SmallRng::new(start_frame as u64),
        })
    }

    /// Stability-thresholded average over the *entire* run so far. Only used
    /// when finalizing a closed run's output mask — NOT for continuity checks,
    /// since it tightens as the run grows (antialiasing fringe pixels drop out
    /// over time) and comparing a live frame against it would make similarity
    /// look like it's declining even when the on-screen text hasn't changed.
    fn average_mask(&self, stability: f64) -> Result<Mat> {
        let mut avg = Mat::default();
        self.mask_accum
            .convert_to(&mut avg, core::CV_32F, 1.0 / self.len as f64, 0.0)?;
        let mut bin = Mat::default();
        imgproc::threshold(&avg, &mut bin, stability, 255.0, imgproc::THRESH_BINARY)?;
        let mut out = Mat::default();
        bin.convert_to(&mut out, core::CV_8U, 1.0, 0.0)?;
        Ok(out)
    }

    fn push(&mut self, mask: Mat, bgr: Mat, max_stack_frames: usize) -> Result<()> {
        let mut mask_f = Mat::default();
        mask.convert_to(&mut mask_f, core::CV_32F, 1.0 / 255.0, 0.0)?;
        let mut new_accum = Mat::default();
        core::add(
            &self.mask_accum,
            &mask_f,
            &mut new_accum,
            &core::no_array(),
            -1,
        )?;
        self.mask_accum = new_accum;
        self.last_mask = mask.try_clone()?;
        self.len += 1;
        self.gap = 0;

        // Reservoir sampling (Algorithm R) so long runs don't buffer every frame.
        if self.bgr_buffer.len() < max_stack_frames {
            self.bgr_buffer.push(bgr);
        } else {
            let j = self.rng.gen_range(self.len);
            if j < max_stack_frames {
                self.bgr_buffer[j] = bgr;
            }
        }
        Ok(())
    }

    fn finalize(&self, stability: f64) -> Result<(Mat, Mat)> {
        let mask = self.average_mask(stability)?;
        let sample = temporal_median_bgr(&self.bgr_buffer)?;
        Ok((mask, sample))
    }
}

/// Per-pixel temporal median across same-sized CV_8UC3 frames. Static content
/// (the subtitle text) stays sharp; content that varies across the run gets
/// pulled toward its most common value instead of averaged/blurred.
fn temporal_median_bgr(frames: &[Mat]) -> Result<Mat> {
    let rows = frames[0].rows();
    let cols = frames[0].cols();
    let channels = 3usize;
    let stride = cols as usize * channels;

    let mut out =
        Mat::new_rows_cols_with_default(rows, cols, core::CV_8UC3, core::Scalar::all(0.0))?;

    let bufs: Vec<&[u8]> = frames
        .iter()
        .map(|m| {
            m.data_bytes()
                .expect("run frames must be continuous CV_8UC3")
        })
        .collect();

    let out_buf = out.data_bytes_mut()?;
    let mut samples = vec![0u8; bufs.len()];

    for r in 0..rows as usize {
        for c in 0..cols as usize {
            for ch in 0..channels {
                let idx = r * stride + c * channels + ch;
                for (i, buf) in bufs.iter().enumerate() {
                    samples[i] = buf[idx];
                }
                samples.sort_unstable();
                out_buf[idx] = samples[samples.len() / 2];
            }
        }
    }
    Ok(out)
}

pub struct SubtitleSearch<I: Iterator<Item = Mat>> {
    frames: I,
    params: Params,
    current_run: Option<Run>,
    frame_idx: usize,
    done: bool,
}

impl<I: Iterator<Item = Mat>> SubtitleSearch<I> {
    pub fn new(frames: I, params: Params) -> Self {
        Self {
            frames,
            params,
            current_run: None,
            frame_idx: 0,
            done: false,
        }
    }

    /// Crop to the ROI and produce a binary edge mask (see module docs for the steps).
    /// Returns (mask, cropped_bgr, on_pixel_count).
    fn edge_mask(&self, frame: &Mat) -> Result<(Mat, Mat, i32)> {
        let cropped = Mat::roi(frame, self.params.roi)?.try_clone()?;

        let mut gray = Mat::default();
        imgproc::cvt_color(
            &cropped,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )?;

        let mut grad_x = Mat::default();
        let mut grad_y = Mat::default();
        imgproc::sobel(
            &gray,
            &mut grad_x,
            core::CV_16S,
            1,
            0,
            3,
            1.0,
            0.0,
            core::BORDER_DEFAULT,
        )?;
        imgproc::sobel(
            &gray,
            &mut grad_y,
            core::CV_16S,
            0,
            1,
            3,
            1.0,
            0.0,
            core::BORDER_DEFAULT,
        )?;

        let mut abs_x = Mat::default();
        let mut abs_y = Mat::default();
        core::convert_scale_abs(&grad_x, &mut abs_x, 1.0, 0.0)?;
        core::convert_scale_abs(&grad_y, &mut abs_y, 1.0, 0.0)?;

        let mut grad = Mat::default();
        core::add_weighted(&abs_x, 0.5, &abs_y, 0.5, 0.0, &mut grad, -1)?;

        let mut mask = Mat::default();
        imgproc::threshold(
            &grad,
            &mut mask,
            self.params.sobel_thresh,
            255.0,
            imgproc::THRESH_BINARY,
        )?;

        let kernel = imgproc::get_structuring_element(
            imgproc::MORPH_RECT,
            core::Size::new(3, 3),
            core::Point::new(-1, -1),
        )?;
        let mut closed = Mat::default();
        imgproc::morphology_ex(
            &mask,
            &mut closed,
            imgproc::MORPH_CLOSE,
            &kernel,
            core::Point::new(-1, -1),
            1,
            core::BORDER_CONSTANT,
            imgproc::morphology_default_border_value()?,
        )?;

        let pixel_count = core::count_non_zero(&closed)?;
        Ok((closed, cropped, pixel_count))
    }

    fn similarity(a: &Mat, b: &Mat) -> Result<f64> {
        let mut inter = Mat::default();
        let mut union = Mat::default();
        core::bitwise_and(a, b, &mut inter, &core::no_array())?;
        core::bitwise_or(a, b, &mut union, &core::no_array())?;

        let union_count = core::count_non_zero(&union)?;
        if union_count == 0 {
            return Ok(0.0);
        }
        let inter_count = core::count_non_zero(&inter)?;
        Ok(inter_count as f64 / union_count as f64)
    }
}

impl<I: Iterator<Item = Mat>> Iterator for SubtitleSearch<I> {
    type Item = SubtitleEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            dbg!(self.current_run.as_ref().map(|x| x.len));
            let frame = match self.frames.next() {
                Some(f) => f,
                None => {
                    self.done = true;
                    if let Some(run) = self.current_run.take() {
                        if run.len >= self.params.min_run_len {
                            let (mask, sample) =
                                run.finalize(self.params.mask_stability_thresh).ok()?;
                            return Some(SubtitleEvent {
                                start_frame: run.start_frame,
                                end_frame: self.frame_idx.saturating_sub(1),
                                sample_bgr: sample,
                                mask,
                            });
                        }
                    }
                    return None;
                }
            };

            let idx = self.frame_idx;
            self.frame_idx += 1;

            let (mask, bgr, pixels) = match self.edge_mask(&frame) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let has_text = pixels >= self.params.min_edge_pixels;

            match (self.current_run.is_some(), has_text) {
                (false, true) => {
                    self.current_run = Run::new(idx, mask, bgr).ok();
                }

                (true, true) => {
                    let run = self.current_run.as_ref().unwrap();
                    let sim = Self::similarity(&run.last_mask, &mask).unwrap_or(0.0);

                    if sim >= self.params.similarity_thresh {
                        if let Some(run) = self.current_run.as_mut() {
                            let _ = run.push(mask, bgr, self.params.max_stack_frames);
                        }
                    } else {
                        let old = self.current_run.take().unwrap();
                        let closed_len = old.len;
                        self.current_run = Run::new(idx, mask, bgr).ok();

                        if closed_len >= self.params.min_run_len {
                            if let Ok((mask, sample)) =
                                old.finalize(self.params.mask_stability_thresh)
                            {
                                return Some(SubtitleEvent {
                                    start_frame: old.start_frame,
                                    end_frame: idx - 1,
                                    sample_bgr: sample,
                                    mask,
                                });
                            }
                        }
                    }
                }

                (true, false) => {
                    if let Some(run) = self.current_run.as_mut() {
                        run.gap += 1;
                    }
                    let gap_exceeded = self
                        .current_run
                        .as_ref()
                        .map(|r| r.gap > self.params.max_gap)
                        .unwrap_or(false);

                    if gap_exceeded {
                        let old = self.current_run.take().unwrap();
                        if old.len >= self.params.min_run_len {
                            if let Ok((mask, sample)) =
                                old.finalize(self.params.mask_stability_thresh)
                            {
                                return Some(SubtitleEvent {
                                    start_frame: old.start_frame,
                                    end_frame: idx - 1,
                                    sample_bgr: sample,
                                    mask,
                                });
                            }
                        }
                    }
                }

                (false, false) => {}
            }
        }
    }
}

// --- Example wiring against a real video, mirroring your `frames.next()` API ---
//
// use opencv::videoio::{VideoCapture, VideoCaptureTrait, CAP_ANY};
//
// struct FrameIter(VideoCapture);
// impl Iterator for FrameIter {
//     type Item = Mat;
//     fn next(&mut self) -> Option<Mat> {
//         let mut m = Mat::default();
//         if self.0.read(&mut m).ok()? && !m.empty() { Some(m) } else { None }
//     }
// }
//
// fn main() -> Result<()> {
//     let cap = VideoCapture::from_file("input.mp4", CAP_ANY)?;
//     let frames = FrameIter(cap);
//
//     let mut params = Params::default();
//     params.roi = core::Rect::new(0, 380, 640, 100); // bottom band, tune to your video
//
//     for event in SubtitleSearch::new(frames, params) {
//         println!("subtitle frames [{}, {}]", event.start_frame, event.end_frame);
//         // event.sample_bgr / event.mask feed into clean_for_ocr(...)
//     }
//     Ok(())
// }
