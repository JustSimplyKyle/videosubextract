// SPDX-License-Identifier: MPL-2.0

use std::ffi::CString;
use std::sync::OnceLock;

type RecognizeText = unsafe extern "C" fn(
    image_data: *const u8,
    image_len: usize,
    output: *mut u8,
    output_len: *mut usize,
) -> i32;

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct VTable {
    pub recognize_text: RecognizeText,
}

// The host reads this symbol as a NUL-terminated C string.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static ocr_name: [u8; b"PaddleOCR v6 Medium (plugin example)\0".len()] =
    *b"PaddleOCR v6 Medium (plugin example)\0";

#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static vtable: VTable = VTable { recognize_text };

static ENGINE: OnceLock<Result<ocr_rs::OcrEngine, String>> = OnceLock::new();

fn engine() -> Result<&'static ocr_rs::OcrEngine, String> {
    ENGINE
        .get_or_init(|| {
            ocr_rs::OcrEngine::from_bytes(
                include_bytes!("../../../models/PP-OCRv6_medium_det.mnn"),
                include_bytes!("../../../models/PP-OCRv6_medium_rec.mnn"),
                include_bytes!("../../../models/ppocr_keys_v6_medium.txt"),
                Some(ocr_rs::OcrEngineConfig::default()),
            )
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn recognize(image_data: &[u8]) -> Result<CString, String> {
    let image = image::load_from_memory_with_format(image_data, image::ImageFormat::Png)
        .map_err(|error| format!("failed to decode input PNG: {error}"))?;
    let text = engine()?
        .recognize(&image)
        .map_err(|error| format!("PaddleOCR failed: {error}"))?
        .iter()
        .map(|result| result.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    CString::new(text).map_err(|_| "recognized text contains an interior NUL byte".to_owned())
}

unsafe extern "C" fn recognize_text(
    image_data: *const u8,
    image_len: usize,
    output: *mut u8,
    output_len: *mut usize,
) -> i32 {
    if output_len.is_null() || (image_data.is_null() && image_len != 0) {
        return 1;
    }

    let result = std::panic::catch_unwind(|| {
        let image_data = if image_len == 0 {
            &[]
        } else {
            // SAFETY: The host promises a readable buffer of `image_len` bytes.
            unsafe { std::slice::from_raw_parts(image_data, image_len) }
        };
        recognize(image_data)
    });

    let text = match result {
        Ok(Ok(text)) => text,
        Ok(Err(error)) => {
            eprintln!("paddleocr plugin: {error}");
            return 2;
        }
        Err(_) => return 3,
    };
    let bytes = text.as_bytes_with_nul();

    if output.is_null() {
        // SAFETY: `output_len` was checked above and is writable by ABI contract.
        unsafe { *output_len = bytes.len() };
        return 0;
    }

    // SAFETY: `output_len` was checked above and is readable by ABI contract.
    let capacity = unsafe { *output_len };
    if capacity < bytes.len() {
        // Report the required capacity to callers that support retrying.
        unsafe { *output_len = bytes.len() };
        return 4;
    }

    // SAFETY: The host promises that `output` points to `capacity` writable bytes.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
    unsafe { *output_len = bytes.len() };
    0
}
