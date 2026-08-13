// ANKAI client — native desktop UI shell (Slint, per ADR-0002).
//
// This binary currently only hosts the glass/blur + drag-reorder spike
// (see docs/adr/0002-native-ui-stack.md, "Spike / prototype plan"). It is
// not the real app shell yet — no nav, no windows beyond this one screen.

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    // Prefer the Skia renderer for full-effects mode per ADR-0002. If Skia
    // can't be selected (missing at compile time, or backend init fails at
    // runtime on this machine), fall back to Slint's default backend
    // selection (FemtoVG/OpenGL, then software) rather than hard-erroring —
    // this mirrors the "Potato mode" fallback path the ADR calls for.
    if let Err(err) = slint::BackendSelector::new()
        .renderer_name("skia".into())
        .select()
    {
        eprintln!(
            "ankai-client: Skia renderer unavailable ({err}); falling back to Slint's default backend/renderer selection."
        );
    }

    // Touch ankai-core so the client crate's dependency on it is real, not
    // just declared — this will grow into actual identity/session wiring.
    // No real key material here, just proving the type is reachable from
    // `client` in the same workspace with no FFI/bridge layer.
    let _placeholder_account = ankai_core::identity::AccountId(String::new());

    let app = AppWindow::new()?;
    app.run()
}
