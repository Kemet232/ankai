//! Thin dynamically-loaded libmpv boundary for direct Stremio streams.
//!
//! Release packages place an LGPL-configured libmpv shared library in the
//! platform bundle (macOS `Contents/Frameworks`, Windows beside the `.exe`,
//! Linux beside the executable under `../lib`). Development builds also try
//! the platform loader's normal search path. Users never need to install mpv.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;

use libloading::Library;

type MpvCreate = unsafe extern "C" fn() -> *mut c_void;
type MpvInitialize = unsafe extern "C" fn(*mut c_void) -> c_int;
type MpvSetOptionString = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type MpvCommand = unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int;
type MpvErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type MpvTerminateDestroy = unsafe extern "C" fn(*mut c_void);

pub struct Player {
    handle: *mut c_void,
    command: MpvCommand,
    error_string: MpvErrorString,
    terminate_destroy: MpvTerminateDestroy,
    // Must outlive every symbol and the mpv handle.
    _library: Library,
}

impl Player {
    pub fn new() -> Result<Self, String> {
        let library = load_library()?;
        unsafe {
            let create: MpvCreate = *library.get(b"mpv_create\0").map_err(symbol_error)?;
            let initialize: MpvInitialize =
                *library.get(b"mpv_initialize\0").map_err(symbol_error)?;
            let set_option: MpvSetOptionString = *library
                .get(b"mpv_set_option_string\0")
                .map_err(symbol_error)?;
            let command: MpvCommand = *library.get(b"mpv_command\0").map_err(symbol_error)?;
            let error_string: MpvErrorString =
                *library.get(b"mpv_error_string\0").map_err(symbol_error)?;
            let terminate_destroy: MpvTerminateDestroy = *library
                .get(b"mpv_terminate_destroy\0")
                .map_err(symbol_error)?;
            let handle = create();
            if handle.is_null() {
                return Err("libmpv could not create a player".into());
            }

            for (name, value) in [
                ("config", "no"),
                ("hwdec", "auto-safe"),
                ("vo", "gpu-next"),
                ("force-window", "yes"),
            ] {
                let name = CString::new(name).unwrap();
                let value = CString::new(value).unwrap();
                check(
                    set_option(handle, name.as_ptr(), value.as_ptr()),
                    error_string,
                )?;
            }
            if let Err(err) = check(initialize(handle), error_string) {
                terminate_destroy(handle);
                return Err(err);
            }
            Ok(Self {
                handle,
                command,
                error_string,
                terminate_destroy,
                _library: library,
            })
        }
    }

    pub fn load(&mut self, url: &str) -> Result<(), String> {
        let command = CString::new("loadfile").unwrap();
        let url = CString::new(url).map_err(|_| "stream URL contains a NUL byte")?;
        let replace = CString::new("replace").unwrap();
        let args = [
            command.as_ptr(),
            url.as_ptr(),
            replace.as_ptr(),
            std::ptr::null(),
        ];
        unsafe {
            check(
                (self.command)(self.handle, args.as_ptr()),
                self.error_string,
            )
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe { (self.terminate_destroy)(self.handle) }
    }
}

fn load_library() -> Result<Library, String> {
    let mut candidates = Vec::<PathBuf>::new();
    if let Ok(path) = std::env::var("ANKAI_LIBMPV_PATH") {
        candidates.push(path.into());
    }
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(&exe);
        #[cfg(target_os = "macos")]
        candidates.push(dir.join("../Frameworks/libmpv.dylib"));
        #[cfg(target_os = "windows")]
        candidates.push(dir.join("mpv-2.dll"));
        #[cfg(target_os = "linux")]
        candidates.push(dir.join("../lib/libmpv.so.2"));
    }
    #[cfg(target_os = "macos")]
    candidates.push("libmpv.dylib".into());
    #[cfg(target_os = "windows")]
    candidates.push("mpv-2.dll".into());
    #[cfg(target_os = "linux")]
    candidates.extend(["libmpv.so.2".into(), "libmpv.so.1".into()]);

    let mut failures = Vec::new();
    for candidate in candidates {
        match unsafe { Library::new(&candidate) } {
            Ok(library) => return Ok(library),
            Err(err) => failures.push(format!("{}: {err}", candidate.display())),
        }
    }
    Err(format!(
        "bundled libmpv is unavailable ({})",
        failures.join("; ")
    ))
}

fn symbol_error(error: libloading::Error) -> String {
    format!("invalid libmpv library: {error}")
}

unsafe fn check(code: c_int, error_string: MpvErrorString) -> Result<(), String> {
    if code >= 0 {
        return Ok(());
    }
    let message = CStr::from_ptr(error_string(code)).to_string_lossy();
    Err(format!("libmpv error: {message}"))
}
