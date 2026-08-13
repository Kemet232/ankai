// ANKAI client — native desktop UI shell (Slint, per ADR-0002).
//
// Default entry point: a minimal window + navigation skeleton (sidebar with
// placeholder destinations, swappable content pane). This is intentionally
// bare. Per ADR-0002's "Spike / prototype plan" gate
// (docs/adr/0002-native-ui-stack.md), native-shell work must not go much
// beyond a window + navigation skeleton until two spikes land: (1) low-end-
// hardware frame-time profiling and (2) accessibility validation with real
// screen readers. Neither is done yet — see that ADR's "Spike 1 results"
// section and PROGRESS.md. Do not add visual polish (glass/blur,
// translucency, animation) here until that gate clears; see
// client/ui/app.slint for the full rationale.
//
// Legacy/debug path: the throwaway glass/blur + drag-reorder rendering
// spike from ADR-0002 spike 1 is still available for reference, gated
// behind `--spike` (or ANKAI_SPIKE_DEBUG=1), since it's not the default
// shell anymore. See client/ui/spike-glass-blur.slint — it is not part of
// the real app.

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

    let spike_debug = std::env::args().any(|arg| arg == "--spike")
        || std::env::var("ANKAI_SPIKE_DEBUG").is_ok_and(|v| v == "1");

    if spike_debug {
        // Legacy/debug path only — see module doc comment above and
        // client/ui/spike-glass-blur.slint.
        let spike = SpikeGlassBlurWindow::new()?;
        return spike.run();
    }

    let app = AppWindow::new()?;
    app.run()
}
