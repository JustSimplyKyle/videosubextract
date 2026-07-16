use eyre::Result;
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static PADDLE_OCR_V6_MEDIUM: OnceLock<Result<ocr_rs::OcrEngine>> = OnceLock::new();
static PADDLE_OCR_V5_MOBILE: OnceLock<Result<ocr_rs::OcrEngine>> = OnceLock::new();

#[derive(Debug, Default, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub enum OcrModel {
    #[default]
    PaddleOcrV6Medium,
    PaddleOcrV5Mobile,
}

impl OcrModel {
    pub const ALL: [Self; 2] = [Self::PaddleOcrV6Medium, Self::PaddleOcrV5Mobile];
    pub const LABELS: [&'static str; 2] = ["PaddleOCR v6 Medium", "PaddleOCR v5 Mobile"];

    fn engine(self) -> Result<&'static ocr_rs::OcrEngine> {
        let engine = match self {
            Self::PaddleOcrV6Medium => PADDLE_OCR_V6_MEDIUM.get_or_init(|| {
                create_engine(
                    include_bytes!("../models/PP-OCRv6_medium_det.mnn"),
                    include_bytes!("../models/PP-OCRv6_medium_rec.mnn"),
                    include_bytes!("../models/ppocr_keys_v6_medium.txt"),
                )
            }),
            Self::PaddleOcrV5Mobile => PADDLE_OCR_V5_MOBILE.get_or_init(|| {
                create_engine(
                    include_bytes!("../models/PP-OCRv5_mobile_det.mnn"),
                    include_bytes!("../models/PP-OCRv5_mobile_rec.mnn"),
                    include_bytes!("../models/ppocr_keys_v5.txt"),
                )
            }),
        };

        engine.as_ref().map_err(|error| eyre::eyre!(error))
    }
}

pub trait OcrProvider: Send + Sync {
    fn recognize_text(&self, image: &DynamicImage) -> Result<String>;
}

fn create_engine(
    detection_model: &'static [u8],
    recognition_model: &'static [u8],
    charset: &'static [u8],
) -> Result<ocr_rs::OcrEngine> {
    ocr_rs::OcrEngine::from_bytes(
        detection_model,
        recognition_model,
        charset,
        Some(ocr_rs::OcrEngineConfig {
            det_options: ocr_rs::DetOptions::default(),
            rec_options: ocr_rs::RecOptions::default(),
            enable_parallel: true,
            backend: ocr_rs::Backend::Vulkan,
            ..Default::default()
        }),
    )
    .map_err(Into::into)
}

impl OcrProvider for OcrModel {
    fn recognize_text(&self, image: &DynamicImage) -> Result<String> {
        Ok(self
            .engine()?
            .recognize(image)?
            .iter()
            .map(|result| result.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"))
    }
}
