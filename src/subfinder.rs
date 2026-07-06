//! Rust approximation of VideoSubFinder's "Run Search" + basic OCR-prep cleanup.
//!
//! Per frame: crop to subtitle ROI -> grayscale -> Sobel edges -> threshold -> morph close.
//! Frames are grouped into runs by comparing each new edge mask against a periodically
//! refreshed *anchor* mask (not a whole-run running average, and not just the immediately
//! previous frame). Two things went wrong with the simpler approaches first:
//!   - comparing to a whole-run running average tightens over time as antialiasing fringe
//!     drops out, so IoU decays even for an unchanged subtitle, spuriously splitting long
//!     runs into fragments too short to pass `min_run_len` (silently dropping them).
//!   - comparing only to the immediately previous frame is too permissive when subtitles
//!     change with no intervening "no text" frame (e.g. continuous dialogue): a genuine
//!     content change can drift past frame-to-frame similarity one small step at a time
//!     without ever tripping the threshold, merging several distinct subtitles into one
//!     giant run.
//! The anchor snaps to the current mask every `anchor_refresh_frames` frames, so a real
//! change is always compared against a reference at most that many frames stale — it can't
//! hide by changing gradually, but it also doesn't decay over an arbitrarily long run.
//! `max_run_len` is an optional hard cap as a belt-and-suspenders safety valve.
//!
//! On footage where the background behind the subtitle moves (anime, live action
//! with a busy shot) Sobel alone isn't enough: it happily finds strong, stable
//! edges in the scene itself — a character's collar outline, a silhouette — and
//! if that edge holds steady for a few frames it satisfies every run-continuity
//! check even though there's no text there. `filter_glyph_like` (see below)
//! rejects connected components that aren't shaped like subtitle glyphs (wrong
//! height, too wide, or not enough of them) before the mask is used for
//! anything else. This helps a lot against single large continuous scene edges;
//! it won't help against background clutter that happens to be glyph-sized
//! (small sparkles, texture) — if that turns out to matter, the next lever is
//! color filtering (isolating pixels matching typical subtitle colors before
//! edge detection even runs), which the real VideoSubFinder leans on more than
//! this shape heuristic does.
//!
//! When a run closes:
//!   - `mask`      = per-pixel "was this an edge in >= stability% of the run's frames"
//!                   (drops transient background edges, keeps stable text strokes)
//!   - `sample_bgr`= per-pixel temporal median of buffered run frames
//!                   (static text stays sharp, moving background gets smoothed out)
//!
//! Set `Params::debug_dir` to dump every run to its own folder — every 10th frame's
//! cropped/mask images plus a SUMMARY.txt saying KEPT/DISCARDED and why — so you
//! can see exactly which frame broke continuity or timed out a gap, instead of
//! just knowing a subtitle went missing (or, didn't stop).
//!
//! Still missing vs. the real thing: color-space filtering before edge detection,
//! and any actual OCR. Those are separate stages layered on top of this one.

use opencv::{Result, core, imgcodecs, imgproc, prelude::*};
use std::path::PathBuf;

/// An inclusive HSV box (OpenCV's 8-bit HSV: H in [0,180], S/V in [0,255]).
/// Used to isolate pixels matching a subtitle's expected fill color before
/// edge detection, so background clutter of the wrong color never gets a
/// chance to look glyph-shaped in the first place.
#[derive(Debug, Clone, Copy)]
pub struct ColorRange {
    pub h_min: f64,
    pub h_max: f64,
    pub s_min: f64,
    pub s_max: f64,
    pub v_min: f64,
    pub v_max: f64,
}

impl ColorRange {
    /// Near-white / light-gray text: ignores hue entirely, just requires low
    /// saturation and high brightness. Covers the large majority of subtitle
    /// styles (plain white, and white-with-outline once you consider the
    /// dilation applied afterward).
    pub fn near_white() -> Self {
        Self {
            h_min: 0.0,
            h_max: 180.0,
            s_min: 0.0,
            s_max: 60.0,
            v_min: 180.0,
            v_max: 255.0,
        }
    }

    /// Bright yellow text, common for karaoke/emphasis styling.
    pub fn near_yellow() -> Self {
        Self {
            h_min: 20.0,
            h_max: 40.0,
            s_min: 80.0,
            s_max: 255.0,
            v_min: 180.0,
            v_max: 255.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Params {
    /// Subtitle search region within each frame (e.g. bottom ~20% of the image).
    pub roi: core::Rect,
    /// Color ranges (in HSV) that count as "subtitle-colored". Defaults to
    /// near-white only; add more (e.g. `ColorRange::near_yellow()`) if your
    /// subtitles use other fill colors. A pixel counts if it falls in *any*
    /// of these ranges.
    pub color_ranges: Vec<ColorRange>,
    /// How many pixels to dilate the color mask by before intersecting it
    /// with the edge mask. Needed because Sobel edges sit right at the
    /// boundary of the color-filled region (and outside any outline ring
    /// around it), not inside it — without dilation, real text edges would
    /// get cut by a color mask that's technically correct but too tight.
    pub color_dilate_px: i32,
    /// Sobel-magnitude threshold used to binarize edges.
    pub sobel_thresh: f64,
    /// Below this many "on" pixels (after the glyph-shape filter) in the mask,
    /// a frame is treated as "no text".
    pub min_edge_pixels: i32,
    /// Connected components shorter than this (in pixels, within the cropped
    /// ROI) are rejected as too small to be a glyph. Set below your smallest
    /// expected font's stroke/counter height.
    pub min_glyph_height: i32,
    /// Connected components taller than this are rejected — real background
    /// edges (character outlines, scene geometry) are very often taller than
    /// any single line of subtitle text. Set a bit above your largest expected
    /// font size.
    pub max_glyph_height: i32,
    /// Connected components wider than this fraction of the ROI's width are
    /// rejected. A single glyph shouldn't span a large fraction of the line;
    /// a long continuous curve (collar outline, horizon line, etc.) usually
    /// will. 0.3 means "wider than 30% of the ROI width gets rejected".
    pub max_glyph_width_frac: f64,
    /// Minimum number of glyph-shaped components required before a frame
    /// counts as "text present". A real subtitle line is several separate
    /// letters/words; a single stray edge passing the size filters by luck
    /// is still just one component and gets rejected by this.
    pub min_glyph_components: i32,
    /// IoU between a new frame's mask and the run's current anchor mask needed
    /// to say "same subtitle continues". The anchor refreshes periodically (see
    /// `anchor_refresh_frames`) rather than being either the very first frame
    /// or the immediately previous one.
    pub similarity_thresh: f64,
    /// How many frames the anchor mask stays fixed before snapping to the
    /// current frame. Smaller = catches subtitle changes faster (less chance
    /// of merging two different lines) but is more sensitive to single-frame
    /// noise; larger = smoother but slower to notice a real change.
    pub anchor_refresh_frames: usize,
    /// Minimum consecutive frames for a run to count as a real subtitle.
    pub min_run_len: usize,
    /// Consecutive "no text" frames tolerated inside a run before closing it.
    pub max_gap: usize,
    /// Optional hard cap on run length as a safety valve: if a run somehow
    /// keeps passing the similarity check for this many frames, force-close
    /// it anyway. A well-tuned anchor should rarely hit this; treat it hitting
    /// as a sign `similarity_thresh` or `anchor_refresh_frames` needs tuning,
    /// not as the primary fix.
    pub max_run_len: Option<usize>,
    /// Fraction of a run's frames a pixel must be "edge-on" in to survive into
    /// the final cleaned mask.
    pub mask_stability_thresh: f64,
    /// Cap on how many frames get buffered per run for the median stack;
    /// longer runs are reservoir-subsampled instead of buffering everything.
    pub max_stack_frames: usize,
    /// If set, dump per-frame images and a SUMMARY.txt for every 10th run (kept or
    /// discarded) into `debug_dir/run_XXXX/`. Leave `None` for normal use —
    /// this does real disk I/O per frame and is meant for diagnosing why
    /// subtitles are getting skipped, not for production runs.
    pub debug_dir: Option<PathBuf>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            roi: core::Rect::new(0, 0, 0, 0), // caller must size this to the actual frame
            color_ranges: vec![ColorRange::near_white()],
            color_dilate_px: 2,
            sobel_thresh: 60.0,
            min_edge_pixels: 50,
            min_glyph_height: 8,
            max_glyph_height: 48,
            max_glyph_width_frac: 0.3,
            min_glyph_components: 2,
            similarity_thresh: 0.5,
            anchor_refresh_frames: 10,
            min_run_len: 5,
            max_gap: 2,
            max_run_len: None, // consider Some(fps * 8) or so while tuning
            mask_stability_thresh: 0.6,
            max_stack_frames: 40,
            debug_dir: None,
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
    run_id: usize,
    start_frame: usize,
    len: usize,
    gap: usize,
    mask_accum: Mat, // CV_32F running count of "edge on" per pixel, for the final mask
    anchor_mask: Mat, // reference mask for continuity checks, refreshed periodically
    anchor_age: usize, // frames since anchor_mask was last refreshed
    bgr_buffer: Vec<Mat>, // reservoir-capped buffer feeding the median stack
    rng: SmallRng,
}

impl Run {
    fn new(run_id: usize, start_frame: usize, mask: Mat, bgr: Mat) -> Result<Self> {
        let mut mask_f = Mat::default();
        mask.convert_to(&mut mask_f, core::CV_32F, 1.0 / 255.0, 0.0)?;
        let anchor_mask = mask.try_clone()?;
        Ok(Self {
            run_id,
            start_frame,
            len: 1,
            gap: 0,
            mask_accum: mask_f,
            anchor_mask,
            anchor_age: 0,
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

    fn push(
        &mut self,
        mask: Mat,
        bgr: Mat,
        max_stack_frames: usize,
        anchor_refresh_frames: usize,
    ) -> Result<()> {
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
        self.len += 1;
        self.gap = 0;

        self.anchor_age += 1;
        if self.anchor_age >= anchor_refresh_frames {
            // Snap the anchor to "now" rather than letting it drift or decay —
            // this is what stops both the over-splitting and the over-merging
            // failure modes at once.
            self.anchor_mask = mask;
            self.anchor_age = 0;
        }
        // If not refreshing, `mask` is simply dropped here; only the anchor and
        // the accumulator need to persist per-frame, not every raw mask.

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
    next_run_id: usize,
    frame_idx: usize,
    done: bool,
}

impl<I: Iterator<Item = Mat>> SubtitleSearch<I> {
    pub fn new(frames: I, params: Params) -> Self {
        Self {
            frames,
            params,
            current_run: None,
            next_run_id: 0,
            frame_idx: 0,
            done: false,
        }
    }

    /// Builds a binary mask of pixels matching any configured `color_ranges`,
    /// dilated by `color_dilate_px` so it still overlaps the Sobel edges that
    /// sit right at the boundary of (and just outside any outline around) the
    /// color-filled glyph interior.
    fn color_mask(&self, cropped: &Mat) -> Result<Mat> {
        let mut hsv = Mat::default();
        imgproc::cvt_color(
            cropped,
            &mut hsv,
            imgproc::COLOR_BGR2HSV,
            0,
            core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )?;

        let mut combined = Mat::new_rows_cols_with_default(
            cropped.rows(),
            cropped.cols(),
            core::CV_8U,
            core::Scalar::all(0.0),
        )?;

        for range in &self.params.color_ranges {
            let lower = core::Scalar::new(range.h_min, range.s_min, range.v_min, 0.0);
            let upper = core::Scalar::new(range.h_max, range.s_max, range.v_max, 0.0);
            let mut in_range = Mat::default();
            core::in_range(&hsv, &lower, &upper, &mut in_range)?;
            let mut merged = Mat::default();
            core::bitwise_or(&combined, &in_range, &mut merged, &core::no_array())?;
            combined = merged;
        }

        if self.params.color_dilate_px > 0 {
            let k = self.params.color_dilate_px * 2 + 1;
            let kernel = imgproc::get_structuring_element(
                imgproc::MORPH_ELLIPSE,
                core::Size::new(k, k),
                core::Point::new(-1, -1),
            )?;
            let mut dilated = Mat::default();
            imgproc::dilate(
                &combined,
                &mut dilated,
                &kernel,
                core::Point::new(-1, -1),
                1,
                core::BORDER_CONSTANT,
                imgproc::morphology_default_border_value()?,
            )?;
            combined = dilated;
        }

        Ok(combined)
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

        let color_mask = self.color_mask(&cropped)?;
        let mut color_filtered = Mat::default();
        core::bitwise_and(&closed, &color_mask, &mut color_filtered, &core::no_array())?;

        self.filter_glyph_like(&color_filtered)
            .map(|(mask, count)| (mask, cropped, count))
    }

    /// Keeps only connected components in `mask` shaped like subtitle glyphs —
    /// bounded height (matches expected font size), bounded width (a single
    /// character isn't very wide), with at least `min_glyph_components` of them
    /// present. This is what tells a line of text apart from a single long,
    /// continuous edge from moving background content (a character silhouette,
    /// a collar outline, etc.), which Sobel alone can't distinguish — it just
    /// finds strong edges, and doesn't care what shape they form.
    ///
    /// Returns (filtered_mask, "on" pixel count — 0 if the component count
    /// requirement wasn't met, so the frame is treated as "no text").
    fn filter_glyph_like(&self, mask: &Mat) -> Result<(Mat, i32)> {
        let mut labels = Mat::default();
        let mut stats = Mat::default();
        let mut centroids = Mat::default();
        let n_labels = imgproc::connected_components_with_stats(
            mask,
            &mut labels,
            &mut stats,
            &mut centroids,
            8,
            core::CV_32S,
        )?;

        let max_width = (mask.cols() as f64 * self.params.max_glyph_width_frac) as i32;
        let mut filtered = Mat::new_rows_cols_with_default(
            mask.rows(),
            mask.cols(),
            core::CV_8U,
            core::Scalar::all(0.0),
        )?;
        let mut glyph_components = 0;
        let mut glyph_pixels = 0;

        // Label 0 is the background component; real components start at 1.
        for label in 1..n_labels {
            let h = *stats.at_2d::<i32>(label, imgproc::CC_STAT_HEIGHT)?;
            let w = *stats.at_2d::<i32>(label, imgproc::CC_STAT_WIDTH)?;
            let area = *stats.at_2d::<i32>(label, imgproc::CC_STAT_AREA)?;

            let looks_like_glyph = h >= self.params.min_glyph_height
                && h <= self.params.max_glyph_height
                && w <= max_width;

            if looks_like_glyph {
                let mut component_mask = Mat::default();
                core::compare(
                    &labels,
                    &core::Scalar::all(label as f64),
                    &mut component_mask,
                    core::CMP_EQ,
                )?;
                let mut merged = Mat::default();
                core::bitwise_or(&filtered, &component_mask, &mut merged, &core::no_array())?;
                filtered = merged;
                glyph_components += 1;
                glyph_pixels += area;
            }
        }

        if glyph_components < self.params.min_glyph_components {
            // Not enough glyph-shaped pieces to call this "text" — zero it out
            // rather than returning whatever scraps happened to pass the shape
            // check (e.g. one letter-sized fragment of a much bigger curve).
            let empty = Mat::new_rows_cols_with_default(
                mask.rows(),
                mask.cols(),
                core::CV_8U,
                core::Scalar::all(0.0),
            )?;
            return Ok((empty, 0));
        }

        Ok((filtered, glyph_pixels))
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

    /// Ensures `debug_dir/run_XXXX/` exists and returns its path, or `None` if
    /// no debug_dir is configured (or it couldn't be created, logged to stderr).
    fn run_dir(&self, run_id: usize) -> Option<PathBuf> {
        let base = self.params.debug_dir.as_ref()?;
        let dir = base.join(format!("run_{run_id:04}"));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("debug_dir: failed to create {dir:?}: {e}");
            return None;
        }
        Some(dir)
    }

    /// Dumps one frame's cropped BGR crop + edge mask into its run's folder.
    /// `tag` distinguishes why the frame was captured: "start" (opened the run),
    /// "cont" (extended it), or "gap" (no-text frame counted against max_gap).
    fn dump_frame(&self, run_id: usize, frame_idx: usize, cropped: &Mat, mask: &Mat, tag: &str) {
        let Some(dir) = self.run_dir(run_id) else {
            return;
        };
        if !run_id.is_multiple_of(10) {
            return;
        }
        let write_params = core::Vector::<i32>::new();

        let bgr_path = dir.join(format!("frame_{frame_idx:06}_{tag}_bgr.png"));
        if let Err(e) = imgcodecs::imwrite(&bgr_path.to_string_lossy(), cropped, &write_params) {
            eprintln!("debug_dir: failed to write {bgr_path:?}: {e}");
        }

        let mask_path = dir.join(format!("frame_{frame_idx:06}_{tag}_mask.png"));
        if let Err(e) = imgcodecs::imwrite(&mask_path.to_string_lossy(), mask, &write_params) {
            eprintln!("debug_dir: failed to write {mask_path:?}: {e}");
        }
    }

    /// Writes SUMMARY.txt plus the finalized mask/sample for a run that just
    /// closed, whether it was kept or discarded — a discarded run's folder is
    /// exactly as inspectable as a kept one, since that's the whole point.
    fn dump_summary(&self, run: &Run, end_frame: usize, kept: bool) {
        let Some(dir) = self.run_dir(run.run_id) else {
            return;
        };

        let summary = format!(
            "status: {}\nstart_frame: {}\nend_frame: {end_frame}\nlen: {}\nmin_run_len (threshold): {}\n",
            if kept { "KEPT" } else { "DISCARDED" },
            run.start_frame,
            run.len,
            self.params.min_run_len,
        );
        if let Err(e) = std::fs::write(dir.join("SUMMARY.txt"), summary) {
            eprintln!("debug_dir: failed to write SUMMARY.txt in {dir:?}: {e}");
        }

        if let Ok((mask, sample)) = run.finalize(self.params.mask_stability_thresh) {
            let write_params = core::Vector::<i32>::new();
            let _ = imgcodecs::imwrite(
                &dir.join("final_mask.png").to_string_lossy(),
                &mask,
                &write_params,
            );
            let _ = imgcodecs::imwrite(
                &dir.join("final_sample.png").to_string_lossy(),
                &sample,
                &write_params,
            );
        }
    }
}

impl<I: Iterator<Item = Mat>> Iterator for SubtitleSearch<I> {
    type Item = SubtitleEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        let debugging = self.params.debug_dir.is_some();

        loop {
            let Some(frame) = self.frames.next() else {
                self.done = true;
                if let Some(run) = self.current_run.take() {
                    let end_frame = self.frame_idx.saturating_sub(1);
                    let kept = run.len >= self.params.min_run_len;
                    if debugging {
                        self.dump_summary(&run, end_frame, kept);
                    }
                    if kept {
                        let (mask, sample) =
                            run.finalize(self.params.mask_stability_thresh).ok()?;
                        return Some(SubtitleEvent {
                            start_frame: run.start_frame,
                            end_frame,
                            sample_bgr: sample,
                            mask,
                        });
                    }
                }
                return None;
            };

            let idx = self.frame_idx;
            self.frame_idx += 1;

            let Ok((mask, bgr, pixels)) = self.edge_mask(&frame) else {
                continue;
            };

            let has_text = pixels >= self.params.min_edge_pixels;

            match (self.current_run.take(), has_text) {
                (None, true) => {
                    let run_id = self.next_run_id;
                    self.next_run_id += 1;
                    if debugging {
                        self.dump_frame(run_id, idx, &bgr, &mask, "start");
                    }
                    self.current_run = Run::new(run_id, idx, mask, bgr).ok();
                }

                (Some(mut run), true) => {
                    if debugging {
                        self.dump_frame(run.run_id, idx, &bgr, &mask, "cont");
                    }

                    let sim = Self::similarity(&run.anchor_mask, &mask).unwrap_or(0.0);

                    if sim >= self.params.similarity_thresh {
                        if let Err(e) = run.push(
                            mask,
                            bgr,
                            self.params.max_stack_frames,
                            self.params.anchor_refresh_frames,
                        ) {
                            eprintln!("{e}");
                        }

                        let hit_cap = self.params.max_run_len.is_some_and(|cap| run.len >= cap);

                        if hit_cap {
                            if debugging {
                                self.dump_summary(&run, idx, true);
                            }
                            if let Ok((mask, sample)) =
                                run.finalize(self.params.mask_stability_thresh)
                            {
                                return Some(SubtitleEvent {
                                    start_frame: run.start_frame,
                                    end_frame: idx,
                                    sample_bgr: sample,
                                    mask,
                                });
                            }
                        } else {
                            // Put the updated run back since we aren't closing it
                            self.current_run = Some(run);
                        }
                    } else {
                        let closed_len = run.len;
                        let kept = closed_len >= self.params.min_run_len;
                        if debugging {
                            self.dump_summary(&run, idx - 1, kept);
                        }

                        let new_run_id = self.next_run_id;
                        self.next_run_id += 1;
                        if debugging {
                            self.dump_frame(new_run_id, idx, &bgr, &mask, "start");
                        }
                        self.current_run = Run::new(new_run_id, idx, mask, bgr).ok();

                        if kept
                            && let Ok((mask, sample)) =
                                run.finalize(self.params.mask_stability_thresh)
                        {
                            return Some(SubtitleEvent {
                                start_frame: run.start_frame,
                                end_frame: idx - 1,
                                sample_bgr: sample,
                                mask,
                            });
                        }
                    }
                }

                (Some(mut run), false) => {
                    run.gap += 1;
                    if debugging {
                        self.dump_frame(run.run_id, idx, &bgr, &mask, "gap");
                    }

                    if run.gap <= self.params.max_gap {
                        // Gap limit not exceeded yet, put the updated run back
                        self.current_run = Some(run);

                        continue;
                    }

                    let kept = run.len >= self.params.min_run_len;
                    if debugging {
                        self.dump_summary(&run, idx - 1, kept);
                    }
                    if kept
                        && let Ok((mask, sample)) = run.finalize(self.params.mask_stability_thresh)
                    {
                        return Some(SubtitleEvent {
                            start_frame: run.start_frame,
                            end_frame: idx - 1,
                            sample_bgr: sample,
                            mask,
                        });
                    }
                }

                (None, false) => {}
            }
        }
    }
}

// --- Example wiring ---
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
//     params.debug_dir = Some("debug_runs".into()); // inspect every run, kept or not
//
//     for event in SubtitleSearch::new(frames, params) {
//         println!("subtitle frames [{}, {}]", event.start_frame, event.end_frame);
//     }
//     Ok(())
// }
