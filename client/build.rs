fn main() {
    slint_build::compile("ui/app.slint").expect("Slint build failed");

    // Separate, additive demo entry point for the FloatingPanel component
    // (client/ui/floating-panel.slint, client/ui/floating-panel-demo.slint)
    // — see PROGRESS.md session 13. Compiled via `compile_with_output_path`
    // rather than `slint_build::compile()` so it does NOT overwrite the
    // `SLINT_INCLUDE_GENERATED` cargo env var that `slint::include_modules!()`
    // in main.rs relies on for app.slint's real generated types (that macro
    // just expands to `include!(env!("SLINT_INCLUDE_GENERATED"))`, and
    // `compile()`/`compile_with_config()` always overwrite that env var on
    // every call — last call wins — so two `compile()` calls would silently
    // drop app.slint's generated bindings). `compile_with_output_path` skips
    // that env var entirely, so main.rs includes this demo's generated code
    // itself via an explicit `include!(concat!(env!("OUT_DIR"), "/floating_panel_demo.rs"))`
    // in its own module instead.
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"),
    );
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let demo_input = manifest_dir.join("ui/floating-panel-demo.slint");
    let demo_output = out_dir.join("floating_panel_demo.rs");
    slint_build::compile_with_output_path(
        &demo_input,
        &demo_output,
        slint_build::CompilerConfiguration::new(),
    )
    .expect("Slint build failed (floating-panel-demo)");
    println!("cargo:rerun-if-changed=ui/floating-panel-demo.slint");
    println!("cargo:rerun-if-changed=ui/floating-panel.slint");
}
