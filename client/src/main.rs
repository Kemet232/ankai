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

/// Opens (creating if necessary) ANKAI's local encrypted database in this
/// platform's standard app-data directory, keyed by the device-local
/// passphrase from `ankai_core::keychain` (OS secure storage, per
/// ADR-0004). Panics on failure — without a working local DB there's
/// nothing useful the app can do, so there's no graceful degraded mode to
/// fall back to here.
fn open_local_db() -> ankai_core::db::Db {
    let dirs = directories::ProjectDirs::from("com", "ankai", "ANKAI")
        .expect("no valid app data directory for this platform/user");
    std::fs::create_dir_all(dirs.data_dir()).expect("failed to create app data directory");

    let passphrase = ankai_core::keychain::device_db_passphrase()
        .expect("failed to obtain device DB encryption key from OS secure storage");

    ankai_core::db::Db::open(dirs.data_dir().join("ankai.sqlite"), &passphrase)
        .expect("failed to open local encrypted database")
}

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

    let spike_debug = std::env::args().any(|arg| arg == "--spike")
        || std::env::var("ANKAI_SPIKE_DEBUG").is_ok_and(|v| v == "1");

    if spike_debug {
        // Legacy/debug path only — see module doc comment above and
        // client/ui/spike-glass-blur.slint.
        let spike = SpikeGlassBlurWindow::new()?;
        return spike.run();
    }

    let db = std::rc::Rc::new(open_local_db());

    // First-run local identity: generates and persists an account/device
    // id plus a real device signature keypair the first time this device's
    // DB is opened, or reloads the same one on every run after that. See
    // ankai_core::identity's module doc comment for what this is (and
    // isn't) yet — no server-issued account, no account-root identity key.
    let mls_provider =
        ankai_core::mls_provider::AnkaiMlsProvider::load(&db).expect("failed to load MLS storage");
    let device = ankai_core::identity::load_or_create_device(&db, &mls_provider)
        .expect("failed to load or create local device identity");
    mls_provider
        .flush(&db)
        .expect("failed to persist MLS storage");

    let app = AppWindow::new()?;
    app.set_account_id(device.account.0.into());
    app.set_device_id(device.id.0.into());

    let saved_display_name = db
        .get_setting("display_name")
        .expect("failed to read display_name setting")
        .unwrap_or_default();
    app.set_display_name(saved_display_name.into());

    let db_for_save = db.clone();
    app.on_save_display_name(move |name| {
        if let Err(err) = db_for_save.set_setting("display_name", &name) {
            eprintln!("ankai-client: failed to save display name: {err}");
        }
    });

    app.run()
}
