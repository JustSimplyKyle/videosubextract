use eyre::Context;
use eyre::Result;
use eyre::bail;
use image::DynamicImage;
use libloading::Library;
use serde::Deserialize;
use serde::Serialize;
use std::{
    ffi::CString,
    io::Cursor,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

pub(crate) type RecognizeText = unsafe extern "C" fn(
    image_data: *const u8,
    image_len: usize,
    output: *mut u8,
    output_len: *mut usize,
) -> i32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicLibrary {
    pub name: String,

    path: PathBuf,

    #[serde(default)]
    generation: u64,

    #[serde(skip)]
    loaded: Arc<OnceLock<LoadedDynamicLibrary>>,
}

#[derive(Debug)]
pub(crate) struct LoadedDynamicLibrary {
    pub(crate) vtable: VTable,
    pub(crate) _library: Library,
}

impl PartialEq for DynamicLibrary {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.name == other.name && self.generation == other.generation
    }
}

impl Eq for DynamicLibrary {}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct VTable {
    /// With a null `output`, the plugin writes the required size (including
    /// the trailing NUL) to `output_len`. Otherwise, `output_len` initially
    /// contains the buffer capacity and is updated to the number of bytes
    /// written, again including the trailing NUL.
    pub recognize_text: RecognizeText,
}

impl DynamicLibrary {
    fn load(&self) -> Result<&LoadedDynamicLibrary> {
        if let Some(loaded) = self.loaded.get() {
            return Ok(loaded);
        }

        let library = unsafe { Library::new(&self.path) }
            .with_context(|| format!("failed to load {}", self.path.display()))?;

        let vtable = unsafe {
            let symbol = library
                .get::<VTable>(b"vtable\0")
                .context("missing vtable symbol")?;

            *symbol
        };

        let loaded = LoadedDynamicLibrary {
            vtable,
            _library: library,
        };

        Ok(self.loaded.get_or_init(|| loaded))
    }

    pub fn recognize_text(&self, image: &DynamicImage) -> Result<String> {
        let loaded = self.load()?;
        let mut image_data = Cursor::new(Vec::new());
        image
            .write_to(&mut image_data, image::ImageFormat::Png)
            .context("failed to encode image for custom OCR library")?;
        let image_data = image_data.into_inner();

        let mut output_len = 0;
        let status = unsafe {
            (loaded.vtable.recognize_text)(
                image_data.as_ptr(),
                image_data.len(),
                std::ptr::null_mut(),
                &raw mut output_len,
            )
        };

        if status != 0 {
            bail!(
                "custom OCR library {} returned status {status} while querying output length",
                self.path.display()
            );
        }

        if output_len == 0 {
            bail!(
                "custom OCR library {} returned an empty output length",
                self.path.display()
            );
        }

        let mut output = vec![0; output_len];

        let output_capacity = output.len();
        let mut written = output_capacity;
        let status = unsafe {
            (loaded.vtable.recognize_text)(
                image_data.as_ptr(),
                image_data.len(),
                output.as_mut_ptr(),
                &raw mut written,
            )
        };

        if status != 0 {
            bail!(
                "custom OCR library {} returned status {status} while writing output",
                self.path.display()
            );
        }

        if written == 0 || written > output_capacity {
            bail!(
                "custom OCR library {} reported writing {written} bytes into a {output_capacity}-byte buffer",
                self.path.display(),
            );
        }

        output.truncate(written);
        let output = CString::from_vec_with_nul(output)
            .context("custom OCR library returned an invalid NUL-terminated string")?;

        output.into_string().with_context(|| {
            format!(
                "custom OCR library {} returned non-UTF-8 text",
                self.path.display()
            )
        })
    }
}
