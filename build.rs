use std::{env, path::PathBuf};

fn main() {
    let source = PathBuf::from("third_party/videosubfinder-src/Components/Headless");
    assert!(
        source.join("CMakeLists.txt").is_file(),
        "VideoSubFinder submodule is missing; run `git submodule update --init --recursive`"
    );

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

    println!("cargo:rustc-link-lib=dylib=stdc++");
}
