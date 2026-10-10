fn main() {
    // The view tests find elements through Slint's debug info, which only builds outside release.
    let debug_info = std::env::var("PROFILE").as_deref() != Ok("release");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug_info);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile the Slint UI");
}
