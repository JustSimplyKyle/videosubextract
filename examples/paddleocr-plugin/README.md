# PaddleOCR dynamic-library example

This crate implements Videosubextract's custom OCR ABI and wraps `ocr-rs`'s
PaddleOCR v6 Medium engine. It embeds the model and character-set files from the
repository's top-level `models` directory.

Build it from the repository root:

```sh
cargo build --release --manifest-path examples/paddleocr-plugin/Cargo.toml
```

Then use the add button beside **OCR model** and select the resulting library:

- Linux: `examples/paddleocr-plugin/target/release/libpaddleocr_plugin_example.so`
- macOS: `examples/paddleocr-plugin/target/release/libpaddleocr_plugin_example.dylib`
- Windows: `examples/paddleocr-plugin/target/release/paddleocr_plugin_example.dll`

The example uses the CPU backend for portability. A plugin must export two
symbols: `ocr_name`, a NUL-terminated UTF-8 byte array, and `vtable`, a `repr(C)`
table whose `recognize_text` function follows the two-call buffer-size protocol
shown in `src/lib.rs`.

