//! Dynamically-loaded libmpv playback and OpenGL render integration.
use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;

type Create = unsafe extern "C" fn() -> *mut c_void;
type Init = unsafe extern "C" fn(*mut c_void) -> c_int;
type SetOption = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type Command = unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int;
type ErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type Destroy = unsafe extern "C" fn(*mut c_void);
type RenderCreate = unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut RenderParam) -> c_int;
type Render = unsafe extern "C" fn(*mut c_void, *mut RenderParam) -> c_int;
type RenderFree = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct OpenGlInit {
    get_proc: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void>,
    context: *mut c_void,
}
#[repr(C)]
struct OpenGlFbo {
    fbo: c_int,
    width: c_int,
    height: c_int,
    internal_format: c_int,
}

pub struct Player {
    handle: *mut c_void,
    context: *mut c_void,
    command_fn: Command,
    error_string: ErrorString,
    destroy: Destroy,
    render_create: RenderCreate,
    render_fn: Render,
    render_free: RenderFree,
    _library: Library,
}

impl Player {
    pub fn new() -> Result<Self, String> {
        let library = load_library()?;
        unsafe {
            let create: Create = symbol(&library, b"mpv_create\0")?;
            let initialize: Init = symbol(&library, b"mpv_initialize\0")?;
            let set_option: SetOption = symbol(&library, b"mpv_set_option_string\0")?;
            let command_fn = symbol(&library, b"mpv_command\0")?;
            let error_string = symbol(&library, b"mpv_error_string\0")?;
            let destroy: Destroy = symbol(&library, b"mpv_terminate_destroy\0")?;
            let render_create = symbol(&library, b"mpv_render_context_create\0")?;
            let render_fn = symbol(&library, b"mpv_render_context_render\0")?;
            let render_free = symbol(&library, b"mpv_render_context_free\0")?;
            let handle = create();
            if handle.is_null() {
                return Err("libmpv could not create a player".into());
            }
            for (name, value) in [
                ("config", "no"),
                ("hwdec", "auto-safe"),
                ("vo", "libmpv"),
                ("force-window", "no"),
            ] {
                check(
                    set_option(handle, cs(name).as_ptr(), cs(value).as_ptr()),
                    error_string,
                )?;
            }
            if let Err(error) = check(initialize(handle), error_string) {
                destroy(handle);
                return Err(error);
            }
            Ok(Self {
                handle,
                context: std::ptr::null_mut(),
                command_fn,
                error_string,
                destroy,
                render_create,
                render_fn,
                render_free,
                _library: library,
            })
        }
    }

    pub fn setup_opengl(&mut self) -> Result<(), String> {
        if !self.context.is_null() {
            return Ok(());
        }
        let api = cs("opengl");
        let mut init = OpenGlInit {
            get_proc: Some(get_proc_address),
            context: std::ptr::null_mut(),
        };
        let mut params = [
            RenderParam {
                kind: 1,
                data: api.as_ptr() as *mut c_void,
            },
            RenderParam {
                kind: 2,
                data: &mut init as *mut _ as *mut c_void,
            },
            RenderParam {
                kind: 0,
                data: std::ptr::null_mut(),
            },
        ];
        unsafe {
            check(
                (self.render_create)(&mut self.context, self.handle, params.as_mut_ptr()),
                self.error_string,
            )
        }
    }

    pub fn render(&mut self, width: i32, height: i32) -> Result<(), String> {
        if self.context.is_null() {
            return Ok(());
        }
        let mut fbo = OpenGlFbo {
            fbo: 0,
            width,
            height,
            internal_format: 0,
        };
        let mut flip: c_int = 1;
        let mut params = [
            RenderParam {
                kind: 3,
                data: &mut fbo as *mut _ as *mut c_void,
            },
            RenderParam {
                kind: 4,
                data: &mut flip as *mut _ as *mut c_void,
            },
            RenderParam {
                kind: 0,
                data: std::ptr::null_mut(),
            },
        ];
        unsafe {
            check(
                (self.render_fn)(self.context, params.as_mut_ptr()),
                self.error_string,
            )
        }
    }

    pub fn load(&mut self, url: &str) -> Result<(), String> {
        self.command(&["loadfile", url, "replace"])
    }
    pub fn toggle_pause(&mut self) -> Result<(), String> {
        self.command(&["cycle", "pause"])
    }
    pub fn stop(&mut self) -> Result<(), String> {
        self.command(&["stop"])
    }
    fn command(&mut self, values: &[&str]) -> Result<(), String> {
        let strings = values
            .iter()
            .map(|value| CString::new(*value).map_err(|_| "command contains a NUL byte"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut args = strings
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        args.push(std::ptr::null());
        unsafe {
            check(
                (self.command_fn)(self.handle, args.as_ptr()),
                self.error_string,
            )
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_null() {
                (self.render_free)(self.context)
            }
            (self.destroy)(self.handle)
        }
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    library
        .get(name)
        .map(|value| *value)
        .map_err(|error| format!("invalid libmpv library: {error}"))
}
fn cs(value: &str) -> CString {
    CString::new(value).unwrap()
}
#[cfg(unix)]
unsafe extern "C" fn get_proc_address(_: *mut c_void, name: *const c_char) -> *mut c_void {
    libc::dlsym(libc::RTLD_DEFAULT, name)
}
#[cfg(windows)]
unsafe extern "C" fn get_proc_address(_: *mut c_void, name: *const c_char) -> *mut c_void {
    extern "system" {
        fn wglGetProcAddress(name: *const c_char) -> *mut c_void;
    }
    wglGetProcAddress(name)
}

fn load_library() -> Result<Library, String> {
    let mut paths = Vec::<PathBuf>::new();
    if let Ok(path) = std::env::var("ANKAI_LIBMPV_PATH") {
        paths.push(path.into())
    }
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(&exe);
        #[cfg(target_os = "macos")]
        paths.push(dir.join("../Frameworks/libmpv.dylib"));
        #[cfg(target_os = "windows")]
        paths.push(dir.join("mpv-2.dll"));
        #[cfg(target_os = "linux")]
        paths.push(dir.join("../lib/libmpv.so.2"));
    }
    #[cfg(target_os = "macos")]
    paths.extend([
        "/opt/homebrew/lib/libmpv.dylib".into(),
        "/usr/local/lib/libmpv.dylib".into(),
        "libmpv.dylib".into(),
    ]);
    #[cfg(target_os = "windows")]
    paths.push("mpv-2.dll".into());
    #[cfg(target_os = "linux")]
    paths.extend(["libmpv.so.2".into(), "libmpv.so.1".into()]);
    let mut errors = Vec::new();
    for path in paths {
        match unsafe { Library::new(&path) } {
            Ok(library) => return Ok(library),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    Err(format!(
        "bundled libmpv is unavailable ({})",
        errors.join("; ")
    ))
}
unsafe fn check(code: c_int, error_string: ErrorString) -> Result<(), String> {
    if code >= 0 {
        Ok(())
    } else {
        Err(format!(
            "libmpv error: {}",
            CStr::from_ptr(error_string(code)).to_string_lossy()
        ))
    }
}
