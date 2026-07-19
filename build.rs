use std::{env, path::PathBuf, process::Command};

fn main() {
    let source = PathBuf::from("third_party/videosubfinder-src/Components/Headless");
    if !source.join("CMakeLists.txt").is_file() {
        panic!(
            "VideoSubFinder submodule is missing; run `git submodule update --init --recursive`"
        );
    }

    println!("cargo:rerun-if-changed={}", source.display());

    let profile = match env::var("PROFILE").as_deref() {
        Ok("release") => "Release",
        _ => "Debug",
    };
    let destination = cmake::Config::new(&source).profile(profile).build();

    println!(
        "cargo:rustc-link-search=native={}",
        destination.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=videosubfinder_headless");

    // Emit OpenCV and oneTBB's transitive native link flags.
    pkg_config::Config::new()
        .probe("opencv4")
        .expect("VideoSubFinder requires OpenCV 4");
    pkg_config::Config::new()
        .probe("tbb")
        .expect("VideoSubFinder requires oneTBB");

    // wxWidgets does not provide pkg-config metadata. CMake uses wx-config to
    // compile the archive, and this mirrors its library flags for Rust's final
    // executable link.
    let output = Command::new("wx-config")
        .args(["--libs", "base"])
        .output()
        .expect("VideoSubFinder requires wx-config");
    assert!(
        output.status.success(),
        "wx-config failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for flag in String::from_utf8(output.stdout)
        .expect("wx-config emitted non-UTF-8 output")
        .split_whitespace()
    {
        if let Some(path) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={path}");
        } else if let Some(library) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib=dylib={library}");
        }
    }

    println!("cargo:rustc-link-lib=dylib=stdc++");
}
