// Builds the Audio Unit host (src/audio/au_host.m) on macOS: a small
// Objective-C layer over AUAudioUnit that src/audio/au.rs calls.
fn main() {
    println!("cargo:rerun-if-changed=src/audio/au_host.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    cc::Build::new()
        .file("src/audio/au_host.m")
        .flag("-fno-objc-arc")
        .flag("-fobjc-exceptions")
        .flag("-Wno-deprecated-declarations")
        .flag("-Wno-unused-parameter")
        .compile("audioboard_au");
    for framework in [
        "AVFoundation",
        "AudioToolbox",
        "CoreAudioKit",
        "AppKit",
        "Foundation",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
