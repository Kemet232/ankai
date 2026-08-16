//! Dynamically-loaded libmpv playback and OpenGL render integration.
//!
//! `Player` deliberately owns the raw libmpv handle and keeps all data returned
//! by `mpv_wait_event` inside [`Player::poll_events`]. libmpv invalidates an
//! event's pointers on the next call to `mpv_wait_event`, so public events and
//! state only contain copied Rust values.

use libloading::Library;
use std::error::Error;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fmt;
use std::path::PathBuf;

const MPV_FORMAT_NONE: c_int = 0;
const MPV_FORMAT_STRING: c_int = 1;
const MPV_FORMAT_FLAG: c_int = 3;
const MPV_FORMAT_INT64: c_int = 4;
const MPV_FORMAT_DOUBLE: c_int = 5;

const MPV_EVENT_NONE: c_int = 0;
const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_START_FILE: c_int = 6;
const MPV_EVENT_END_FILE: c_int = 7;
const MPV_EVENT_FILE_LOADED: c_int = 8;
const MPV_EVENT_TRACKS_CHANGED: c_int = 9;
const MPV_EVENT_TRACK_SWITCHED: c_int = 10;
const MPV_EVENT_VIDEO_RECONFIG: c_int = 17;
const MPV_EVENT_AUDIO_RECONFIG: c_int = 18;
const MPV_EVENT_SEEK: c_int = 20;
const MPV_EVENT_PLAYBACK_RESTART: c_int = 21;
const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;
const MPV_EVENT_QUEUE_OVERFLOW: c_int = 24;

const MPV_ERROR_PROPERTY_NOT_FOUND: c_int = -8;
const MPV_ERROR_PROPERTY_UNAVAILABLE: c_int = -10;

const MAX_EVENTS_PER_POLL: usize = 256;
const MAX_TRACKS: i64 = 1_024;
const MAX_VOLUME: f64 = 130.0;
const MIN_SPEED: f64 = 0.25;
const MAX_SPEED: f64 = 4.0;

type Create = unsafe extern "C" fn() -> *mut c_void;
type Init = unsafe extern "C" fn(*mut c_void) -> c_int;
type SetOption = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type Command = unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int;
type GetProperty = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int;
type SetProperty = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int;
type ObserveProperty = unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int;
type WaitEvent = unsafe extern "C" fn(*mut c_void, f64) -> *const MpvEvent;
type MpvFree = unsafe extern "C" fn(*mut c_void);
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

#[repr(C)]
struct MpvEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventEndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

/// A user-facing playback phase derived from libmpv events and properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackPhase {
    #[default]
    Idle,
    Loading,
    Buffering,
    Playing,
    Paused,
    Ended,
    Error,
}

/// The media track categories exposed by mpv's `track-list` property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
    Unknown(String),
}

/// A copied, UI-safe representation of one mpv media track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTrack {
    pub id: i64,
    pub kind: TrackKind,
    pub title: Option<String>,
    pub language: Option<String>,
    pub codec: Option<String>,
    pub selected: bool,
    pub default: bool,
    pub forced: bool,
    pub external: bool,
}

/// The current playback values a UI normally needs to paint its controls.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    pub phase: PlaybackPhase,
    pub has_media: bool,
    pub paused: bool,
    pub muted: bool,
    pub seeking: bool,
    pub paused_for_cache: bool,
    pub eof_reached: bool,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub buffering_percent: Option<f64>,
    pub volume: f64,
    pub speed: f64,
    pub media_title: Option<String>,
    pub tracks: Vec<MediaTrack>,
    pub last_error: Option<String>,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            phase: PlaybackPhase::Idle,
            has_media: false,
            paused: false,
            muted: false,
            seeking: false,
            paused_for_cache: false,
            eof_reached: false,
            position_seconds: None,
            duration_seconds: None,
            buffering_percent: None,
            volume: 100.0,
            speed: 1.0,
            media_title: None,
            tracks: Vec::new(),
            last_error: None,
        }
    }
}

impl PlayerState {
    /// Playback progress in the inclusive range `0.0..=1.0`, when known.
    pub fn progress(&self) -> Option<f64> {
        let duration = self.duration_seconds?;
        let position = self.position_seconds?;
        (duration > 0.0).then(|| (position / duration).clamp(0.0, 1.0))
    }

    fn update_active_phase(&mut self) {
        if !self.has_media
            || matches!(
                self.phase,
                PlaybackPhase::Loading | PlaybackPhase::Ended | PlaybackPhase::Error
            )
        {
            return;
        }
        self.phase = if self.paused_for_cache {
            PlaybackPhase::Buffering
        } else if self.paused {
            PlaybackPhase::Paused
        } else {
            PlaybackPhase::Playing
        };
    }

    fn start_loading(&mut self) {
        self.phase = PlaybackPhase::Loading;
        self.has_media = true;
        self.seeking = false;
        self.paused_for_cache = false;
        self.eof_reached = false;
        self.position_seconds = None;
        self.duration_seconds = None;
        self.buffering_percent = None;
        self.media_title = None;
        self.tracks.clear();
        self.last_error = None;
    }

    fn become_idle(&mut self) {
        self.phase = PlaybackPhase::Idle;
        self.has_media = false;
        self.seeking = false;
        self.paused_for_cache = false;
        self.eof_reached = false;
        self.position_seconds = None;
        self.duration_seconds = None;
        self.buffering_percent = None;
        self.media_title = None;
        self.tracks.clear();
    }
}

/// An observed mpv property changed. The latest value lives in [`PlayerState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedProperty {
    Pause,
    Mute,
    TimePosition,
    Duration,
    Volume,
    Speed,
    Seeking,
    PausedForCache,
    BufferingPercent,
    IdleActive,
    EofReached,
    TrackList,
}

/// Why mpv ended the current file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    EndOfFile,
    Stop,
    Quit,
    Error,
    Redirect,
    Unknown(i32),
}

impl From<c_int> for EndReason {
    fn from(value: c_int) -> Self {
        match value {
            0 => Self::EndOfFile,
            2 => Self::Stop,
            3 => Self::Quit,
            4 => Self::Error,
            5 => Self::Redirect,
            value => Self::Unknown(value),
        }
    }
}

/// Owned events suitable for dispatch to Slint after `poll_events` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEvent {
    FileStarted,
    FileLoaded,
    TracksChanged,
    TrackSwitched,
    VideoReconfigured,
    AudioReconfigured,
    Seek,
    PlaybackRestart,
    PropertyChanged(ObservedProperty),
    Ended(EndReason),
    QueueOverflow,
    Shutdown,
    Error(String),
}

/// Errors from library loading, FFI validation, and mpv operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerError {
    LibraryUnavailable(String),
    InvalidLibrary(String),
    CreateFailed,
    InteriorNul {
        context: &'static str,
    },
    InvalidValue {
        name: &'static str,
        value: String,
    },
    Mpv {
        operation: String,
        code: c_int,
        message: String,
    },
}

impl PlayerError {
    fn is_property_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Mpv {
                code: MPV_ERROR_PROPERTY_NOT_FOUND | MPV_ERROR_PROPERTY_UNAVAILABLE,
                ..
            }
        )
    }
}

impl fmt::Display for PlayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LibraryUnavailable(details) => {
                write!(formatter, "bundled libmpv is unavailable ({details})")
            }
            Self::InvalidLibrary(error) => write!(formatter, "invalid libmpv library: {error}"),
            Self::CreateFailed => write!(formatter, "libmpv could not create a player"),
            Self::InteriorNul { context } => write!(formatter, "{context} contains a NUL byte"),
            Self::InvalidValue { name, value } => {
                write!(formatter, "invalid {name} value: {value}")
            }
            Self::Mpv {
                operation, message, ..
            } => {
                write!(formatter, "libmpv {operation} failed: {message}")
            }
        }
    }
}

impl Error for PlayerError {}

pub type PlayerResult<T> = Result<T, PlayerError>;

struct ObservedPropertySpec {
    id: u64,
    property: ObservedProperty,
    name: &'static str,
    format: c_int,
}

const OBSERVED_PROPERTIES: &[ObservedPropertySpec] = &[
    ObservedPropertySpec {
        id: 1,
        property: ObservedProperty::Pause,
        name: "pause",
        format: MPV_FORMAT_FLAG,
    },
    ObservedPropertySpec {
        id: 2,
        property: ObservedProperty::Mute,
        name: "mute",
        format: MPV_FORMAT_FLAG,
    },
    ObservedPropertySpec {
        id: 3,
        property: ObservedProperty::TimePosition,
        name: "time-pos",
        format: MPV_FORMAT_DOUBLE,
    },
    ObservedPropertySpec {
        id: 4,
        property: ObservedProperty::Duration,
        name: "duration",
        format: MPV_FORMAT_DOUBLE,
    },
    ObservedPropertySpec {
        id: 5,
        property: ObservedProperty::Volume,
        name: "volume",
        format: MPV_FORMAT_DOUBLE,
    },
    ObservedPropertySpec {
        id: 6,
        property: ObservedProperty::Speed,
        name: "speed",
        format: MPV_FORMAT_DOUBLE,
    },
    ObservedPropertySpec {
        id: 7,
        property: ObservedProperty::Seeking,
        name: "seeking",
        format: MPV_FORMAT_FLAG,
    },
    ObservedPropertySpec {
        id: 8,
        property: ObservedProperty::PausedForCache,
        name: "paused-for-cache",
        format: MPV_FORMAT_FLAG,
    },
    ObservedPropertySpec {
        id: 9,
        property: ObservedProperty::BufferingPercent,
        name: "cache-buffering-state",
        format: MPV_FORMAT_DOUBLE,
    },
    ObservedPropertySpec {
        id: 10,
        property: ObservedProperty::IdleActive,
        name: "idle-active",
        format: MPV_FORMAT_FLAG,
    },
    ObservedPropertySpec {
        id: 11,
        property: ObservedProperty::EofReached,
        name: "eof-reached",
        format: MPV_FORMAT_FLAG,
    },
    // `track-list` is a nested mpv node. Observe it without a payload and
    // rebuild the copied scalar track list when mpv says it may have changed.
    ObservedPropertySpec {
        id: 12,
        property: ObservedProperty::TrackList,
        name: "track-list",
        format: MPV_FORMAT_NONE,
    },
];

pub struct Player {
    handle: *mut c_void,
    context: *mut c_void,
    state: PlayerState,
    command_fn: Command,
    get_property: GetProperty,
    set_property: SetProperty,
    wait_event: WaitEvent,
    mpv_free: MpvFree,
    error_string: ErrorString,
    destroy: Destroy,
    render_create: RenderCreate,
    render_fn: Render,
    render_free: RenderFree,
    _library: Library,
}

impl Player {
    pub fn new() -> PlayerResult<Self> {
        let library = load_library()?;
        unsafe {
            let create: Create = symbol(&library, b"mpv_create\0")?;
            let initialize: Init = symbol(&library, b"mpv_initialize\0")?;
            let set_option: SetOption = symbol(&library, b"mpv_set_option_string\0")?;
            let command_fn = symbol(&library, b"mpv_command\0")?;
            let get_property = symbol(&library, b"mpv_get_property\0")?;
            let set_property = symbol(&library, b"mpv_set_property\0")?;
            let observe_property: ObserveProperty = symbol(&library, b"mpv_observe_property\0")?;
            let wait_event = symbol(&library, b"mpv_wait_event\0")?;
            let mpv_free = symbol(&library, b"mpv_free\0")?;
            let error_string = symbol(&library, b"mpv_error_string\0")?;
            let destroy: Destroy = symbol(&library, b"mpv_terminate_destroy\0")?;
            let render_create = symbol(&library, b"mpv_render_context_create\0")?;
            let render_fn = symbol(&library, b"mpv_render_context_render\0")?;
            let render_free = symbol(&library, b"mpv_render_context_free\0")?;
            let handle = create();
            if handle.is_null() {
                return Err(PlayerError::CreateFailed);
            }

            let configure = || -> PlayerResult<()> {
                for (name, value) in [
                    ("config", "no"),
                    ("hwdec", "auto-safe"),
                    ("vo", "libmpv"),
                    ("force-window", "no"),
                    ("keep-open", "yes"),
                ] {
                    check(
                        set_option(handle, literal(name).as_ptr(), literal(value).as_ptr()),
                        error_string,
                        format!("setting option {name}"),
                    )?;
                }
                check(initialize(handle), error_string, "initialization")?;
                for spec in OBSERVED_PROPERTIES {
                    check(
                        observe_property(handle, spec.id, literal(spec.name).as_ptr(), spec.format),
                        error_string,
                        format!("observing property {}", spec.name),
                    )?;
                }
                Ok(())
            };

            if let Err(error) = configure() {
                destroy(handle);
                return Err(error);
            }

            Ok(Self {
                handle,
                context: std::ptr::null_mut(),
                state: PlayerState::default(),
                command_fn,
                get_property,
                set_property,
                wait_event,
                mpv_free,
                error_string,
                destroy,
                render_create,
                render_fn,
                render_free,
                _library: library,
            })
        }
    }

    pub fn state(&self) -> &PlayerState {
        &self.state
    }

    pub fn tracks(&self) -> &[MediaTrack] {
        &self.state.tracks
    }

    pub fn setup_opengl(&mut self) -> PlayerResult<()> {
        if !self.context.is_null() {
            return Ok(());
        }
        let api = literal("opengl");
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
                "render-context creation",
            )
        }
    }

    /// Render into the currently bound/default framebuffer.
    pub fn render(&mut self, width: i32, height: i32) -> PlayerResult<()> {
        self.render_to_fbo(0, width, height, 0, true)
    }

    /// Render into an application-owned OpenGL framebuffer.
    ///
    /// This is the backend seam needed for a bounded Slint video surface:
    /// pass the surface FBO instead of allowing mpv to paint the whole window.
    pub fn render_to_fbo(
        &mut self,
        fbo: i32,
        width: i32,
        height: i32,
        internal_format: i32,
        flip_y: bool,
    ) -> PlayerResult<()> {
        if self.context.is_null() {
            return Ok(());
        }
        if width <= 0 || height <= 0 {
            return Err(PlayerError::InvalidValue {
                name: "render size",
                value: format!("{width}x{height}"),
            });
        }
        let mut framebuffer = OpenGlFbo {
            fbo,
            width,
            height,
            internal_format,
        };
        let mut flip = c_int::from(flip_y);
        let mut params = [
            RenderParam {
                kind: 3,
                data: &mut framebuffer as *mut _ as *mut c_void,
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
                "render",
            )
        }
    }

    pub fn load(&mut self, url: &str) -> PlayerResult<()> {
        self.command(&["loadfile", url, "replace"])?;
        self.state.start_loading();
        Ok(())
    }

    pub fn play(&mut self) -> PlayerResult<()> {
        self.set_pause(false)
    }

    pub fn pause(&mut self) -> PlayerResult<()> {
        self.set_pause(true)
    }

    pub fn set_pause(&mut self, paused: bool) -> PlayerResult<()> {
        self.set_property_flag("pause", paused)?;
        self.state.paused = paused;
        self.state.update_active_phase();
        Ok(())
    }

    pub fn toggle_pause(&mut self) -> PlayerResult<()> {
        self.set_pause(!self.state.paused)
    }

    pub fn set_mute(&mut self, muted: bool) -> PlayerResult<()> {
        self.set_property_flag("mute", muted)?;
        self.state.muted = muted;
        Ok(())
    }

    pub fn toggle_mute(&mut self) -> PlayerResult<()> {
        self.set_mute(!self.state.muted)
    }

    pub fn set_volume(&mut self, volume: f64) -> PlayerResult<()> {
        validate_finite_range("volume", volume, 0.0, MAX_VOLUME)?;
        self.set_property_double("volume", volume)?;
        self.state.volume = volume;
        Ok(())
    }

    pub fn adjust_volume(&mut self, delta: f64) -> PlayerResult<()> {
        if !delta.is_finite() {
            return Err(invalid_value("volume delta", delta));
        }
        self.set_volume((self.state.volume + delta).clamp(0.0, MAX_VOLUME))
    }

    pub fn set_speed(&mut self, speed: f64) -> PlayerResult<()> {
        validate_finite_range("playback speed", speed, MIN_SPEED, MAX_SPEED)?;
        self.set_property_double("speed", speed)?;
        self.state.speed = speed;
        Ok(())
    }

    pub fn seek_absolute(&mut self, seconds: f64) -> PlayerResult<()> {
        validate_finite_range("seek position", seconds, 0.0, f64::MAX)?;
        self.command_owned(&["seek".into(), seconds.to_string(), "absolute+exact".into()])
    }

    pub fn seek_relative(&mut self, seconds: f64) -> PlayerResult<()> {
        if !seconds.is_finite() {
            return Err(invalid_value("relative seek", seconds));
        }
        self.command_owned(&["seek".into(), seconds.to_string(), "relative+exact".into()])
    }

    pub fn seek_percent(&mut self, percent: f64) -> PlayerResult<()> {
        validate_finite_range("seek percent", percent, 0.0, 100.0)?;
        self.command_owned(&[
            "seek".into(),
            percent.to_string(),
            "absolute-percent+exact".into(),
        ])
    }

    pub fn set_audio_track(&mut self, track_id: Option<i64>) -> PlayerResult<()> {
        self.set_optional_track("aid", track_id)
    }

    pub fn set_subtitle_track(&mut self, track_id: Option<i64>) -> PlayerResult<()> {
        self.set_optional_track("sid", track_id)
    }

    pub fn set_subtitle_delay(&mut self, seconds: f64) -> PlayerResult<()> {
        if !seconds.is_finite() {
            return Err(invalid_value("subtitle delay", seconds));
        }
        self.set_property_double("sub-delay", seconds)
    }

    pub fn stop(&mut self) -> PlayerResult<()> {
        self.command(&["stop"])?;
        self.state.become_idle();
        Ok(())
    }

    /// Execute a synchronous mpv command after validating every argument.
    pub fn command(&mut self, values: &[&str]) -> PlayerResult<()> {
        let owned = values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        self.command_owned(&owned)
    }

    pub fn get_property_flag(&self, name: &str) -> PlayerResult<Option<bool>> {
        let name = ffi_string(name, "property name")?;
        let mut value: c_int = 0;
        let result = unsafe {
            check(
                (self.get_property)(
                    self.handle,
                    name.as_ptr(),
                    MPV_FORMAT_FLAG,
                    &mut value as *mut _ as *mut c_void,
                ),
                self.error_string,
                format!("reading property {}", name.to_string_lossy()),
            )
        };
        optional_property(result.map(|()| value != 0))
    }

    pub fn get_property_double(&self, name: &str) -> PlayerResult<Option<f64>> {
        let name = ffi_string(name, "property name")?;
        let mut value = 0.0;
        let result = unsafe {
            check(
                (self.get_property)(
                    self.handle,
                    name.as_ptr(),
                    MPV_FORMAT_DOUBLE,
                    &mut value as *mut _ as *mut c_void,
                ),
                self.error_string,
                format!("reading property {}", name.to_string_lossy()),
            )
        };
        optional_property(result.map(|()| value))
    }

    pub fn get_property_i64(&self, name: &str) -> PlayerResult<Option<i64>> {
        let name = ffi_string(name, "property name")?;
        let mut value = 0_i64;
        let result = unsafe {
            check(
                (self.get_property)(
                    self.handle,
                    name.as_ptr(),
                    MPV_FORMAT_INT64,
                    &mut value as *mut _ as *mut c_void,
                ),
                self.error_string,
                format!("reading property {}", name.to_string_lossy()),
            )
        };
        optional_property(result.map(|()| value))
    }

    pub fn get_property_string(&self, name: &str) -> PlayerResult<Option<String>> {
        let name = ffi_string(name, "property name")?;
        let mut value: *mut c_char = std::ptr::null_mut();
        let result = unsafe {
            check(
                (self.get_property)(
                    self.handle,
                    name.as_ptr(),
                    MPV_FORMAT_STRING,
                    &mut value as *mut _ as *mut c_void,
                ),
                self.error_string,
                format!("reading property {}", name.to_string_lossy()),
            )
        };
        match optional_property(result)? {
            None => Ok(None),
            Some(()) if value.is_null() => Ok(None),
            Some(()) => unsafe {
                let copied = CStr::from_ptr(value).to_string_lossy().into_owned();
                (self.mpv_free)(value as *mut c_void);
                Ok(Some(copied))
            },
        }
    }

    pub fn set_property_flag(&mut self, name: &str, value: bool) -> PlayerResult<()> {
        let mut value = c_int::from(value);
        self.set_property_raw(name, MPV_FORMAT_FLAG, &mut value as *mut _ as *mut c_void)
    }

    pub fn set_property_double(&mut self, name: &str, value: f64) -> PlayerResult<()> {
        if !value.is_finite() {
            return Err(PlayerError::InvalidValue {
                name: "property",
                value: format!("{name}={value}"),
            });
        }
        let mut value = value;
        self.set_property_raw(name, MPV_FORMAT_DOUBLE, &mut value as *mut _ as *mut c_void)
    }

    pub fn set_property_i64(&mut self, name: &str, value: i64) -> PlayerResult<()> {
        let mut value = value;
        self.set_property_raw(name, MPV_FORMAT_INT64, &mut value as *mut _ as *mut c_void)
    }

    pub fn set_property_string(&mut self, name: &str, value: &str) -> PlayerResult<()> {
        let value = ffi_string(value, "property value")?;
        // MPV_FORMAT_STRING is represented as `char *`, and mpv_set_property
        // expects a pointer to that value (`char **`), not the bytes directly.
        let mut value_pointer = value.as_ptr();
        self.set_property_raw(
            name,
            MPV_FORMAT_STRING,
            &mut value_pointer as *mut _ as *mut c_void,
        )
    }

    /// Refresh all inexpensive control properties and the track list.
    pub fn refresh_state(&mut self) -> PlayerResult<&PlayerState> {
        if let Some(value) = self.get_property_flag("pause")? {
            self.state.paused = value;
        }
        if let Some(value) = self.get_property_flag("mute")? {
            self.state.muted = value;
        }
        self.state.position_seconds = self.get_property_double("time-pos")?;
        self.state.duration_seconds = self.get_property_double("duration")?;
        if let Some(value) = self.get_property_double("volume")? {
            self.state.volume = value;
        }
        if let Some(value) = self.get_property_double("speed")? {
            self.state.speed = value;
        }
        if let Some(value) = self.get_property_flag("seeking")? {
            self.state.seeking = value;
        }
        if let Some(value) = self.get_property_flag("paused-for-cache")? {
            self.state.paused_for_cache = value;
        }
        self.state.buffering_percent = self
            .get_property_double("cache-buffering-state")?
            .map(|value| value.clamp(0.0, 100.0));
        self.state.media_title = self.get_property_string("media-title")?;
        self.refresh_tracks()?;
        self.state.update_active_phase();
        Ok(&self.state)
    }

    /// Drain currently queued mpv events without blocking the UI thread.
    pub fn poll_events(&mut self) -> PlayerResult<Vec<PlayerEvent>> {
        let mut events = Vec::new();
        for _ in 0..MAX_EVENTS_PER_POLL {
            let event = unsafe { (self.wait_event)(self.handle, 0.0) };
            if event.is_null() {
                break;
            }
            let event = unsafe { &*event };
            if event.event_id == MPV_EVENT_NONE {
                break;
            }
            if event.error < 0 {
                let message = unsafe { error_message(self.error_string, event.error) };
                self.state.phase = PlaybackPhase::Error;
                self.state.last_error = Some(message.clone());
                events.push(PlayerEvent::Error(message));
            }
            self.copy_event(event, &mut events)?;
        }
        Ok(events)
    }

    fn copy_event(&mut self, event: &MpvEvent, events: &mut Vec<PlayerEvent>) -> PlayerResult<()> {
        match event.event_id {
            MPV_EVENT_START_FILE => {
                self.state.start_loading();
                events.push(PlayerEvent::FileStarted);
            }
            MPV_EVENT_FILE_LOADED => {
                self.state.has_media = true;
                self.state.media_title = self.get_property_string("media-title")?;
                self.refresh_tracks()?;
                self.state.phase = if self.state.paused_for_cache {
                    PlaybackPhase::Buffering
                } else if self.state.paused {
                    PlaybackPhase::Paused
                } else {
                    PlaybackPhase::Playing
                };
                events.push(PlayerEvent::FileLoaded);
            }
            MPV_EVENT_END_FILE => self.copy_end_file(event.data, events),
            MPV_EVENT_TRACKS_CHANGED => {
                self.refresh_tracks()?;
                events.push(PlayerEvent::TracksChanged);
            }
            MPV_EVENT_TRACK_SWITCHED => {
                self.refresh_tracks()?;
                events.push(PlayerEvent::TrackSwitched);
            }
            MPV_EVENT_VIDEO_RECONFIG => events.push(PlayerEvent::VideoReconfigured),
            MPV_EVENT_AUDIO_RECONFIG => events.push(PlayerEvent::AudioReconfigured),
            MPV_EVENT_SEEK => {
                self.state.seeking = true;
                events.push(PlayerEvent::Seek);
            }
            MPV_EVENT_PLAYBACK_RESTART => {
                self.state.seeking = false;
                self.state.has_media = true;
                self.state.phase = if self.state.paused_for_cache {
                    PlaybackPhase::Buffering
                } else if self.state.paused {
                    PlaybackPhase::Paused
                } else {
                    PlaybackPhase::Playing
                };
                events.push(PlayerEvent::PlaybackRestart);
            }
            MPV_EVENT_PROPERTY_CHANGE => self.copy_property_change(event, events)?,
            MPV_EVENT_QUEUE_OVERFLOW => events.push(PlayerEvent::QueueOverflow),
            MPV_EVENT_SHUTDOWN => {
                self.state.become_idle();
                events.push(PlayerEvent::Shutdown);
            }
            _ => {}
        }
        Ok(())
    }

    fn copy_end_file(&mut self, data: *mut c_void, events: &mut Vec<PlayerEvent>) {
        let (reason, error) = if data.is_null() {
            (EndReason::Unknown(-1), 0)
        } else {
            let event = unsafe { &*(data as *const MpvEventEndFile) };
            (EndReason::from(event.reason), event.error)
        };
        match reason {
            EndReason::EndOfFile => {
                self.state.phase = PlaybackPhase::Ended;
                self.state.eof_reached = true;
            }
            EndReason::Stop | EndReason::Quit => self.state.become_idle(),
            EndReason::Redirect => self.state.start_loading(),
            EndReason::Error => {
                let message = if error < 0 {
                    unsafe { error_message(self.error_string, error) }
                } else {
                    "playback ended with an unknown mpv error".to_owned()
                };
                self.state.phase = PlaybackPhase::Error;
                self.state.last_error = Some(message.clone());
                events.push(PlayerEvent::Error(message));
            }
            EndReason::Unknown(_) => self.state.phase = PlaybackPhase::Ended,
        }
        events.push(PlayerEvent::Ended(reason));
    }

    fn copy_property_change(
        &mut self,
        event: &MpvEvent,
        events: &mut Vec<PlayerEvent>,
    ) -> PlayerResult<()> {
        if event.data.is_null() {
            return Ok(());
        }
        let raw = unsafe { &*(event.data as *const MpvEventProperty) };
        let Some(property) = observed_property(event.reply_userdata, raw.name) else {
            return Ok(());
        };
        if property == ObservedProperty::TrackList {
            self.refresh_tracks()?;
            events.push(PlayerEvent::TracksChanged);
            return Ok(());
        }
        match raw.format {
            MPV_FORMAT_FLAG if !raw.data.is_null() => {
                let value = unsafe { *(raw.data as *const c_int) } != 0;
                self.apply_flag_property(property, value);
            }
            MPV_FORMAT_DOUBLE if !raw.data.is_null() => {
                let value = unsafe { *(raw.data as *const f64) };
                self.apply_double_property(property, value);
            }
            MPV_FORMAT_NONE => self.apply_unavailable_property(property),
            _ => return Ok(()),
        }
        events.push(PlayerEvent::PropertyChanged(property));
        Ok(())
    }

    fn apply_flag_property(&mut self, property: ObservedProperty, value: bool) {
        match property {
            ObservedProperty::Pause => self.state.paused = value,
            ObservedProperty::Mute => self.state.muted = value,
            ObservedProperty::Seeking => self.state.seeking = value,
            ObservedProperty::PausedForCache => self.state.paused_for_cache = value,
            ObservedProperty::IdleActive if value => self.state.become_idle(),
            ObservedProperty::EofReached => {
                self.state.eof_reached = value;
                if value {
                    self.state.phase = PlaybackPhase::Ended;
                }
            }
            _ => {}
        }
        self.state.update_active_phase();
    }

    fn apply_double_property(&mut self, property: ObservedProperty, value: f64) {
        if !value.is_finite() {
            return;
        }
        match property {
            ObservedProperty::TimePosition => self.state.position_seconds = Some(value.max(0.0)),
            ObservedProperty::Duration => {
                self.state.duration_seconds = (value > 0.0).then_some(value)
            }
            ObservedProperty::Volume => self.state.volume = value.max(0.0),
            ObservedProperty::Speed if value > 0.0 => self.state.speed = value,
            ObservedProperty::BufferingPercent => {
                self.state.buffering_percent = Some(value.clamp(0.0, 100.0))
            }
            _ => {}
        }
    }

    fn apply_unavailable_property(&mut self, property: ObservedProperty) {
        match property {
            ObservedProperty::TimePosition => self.state.position_seconds = None,
            ObservedProperty::Duration => self.state.duration_seconds = None,
            ObservedProperty::BufferingPercent => self.state.buffering_percent = None,
            _ => {}
        }
    }

    fn refresh_tracks(&mut self) -> PlayerResult<()> {
        let count = self.get_property_i64("track-list/count")?.unwrap_or(0);
        if !(0..=MAX_TRACKS).contains(&count) {
            return Err(PlayerError::InvalidValue {
                name: "track count",
                value: count.to_string(),
            });
        }
        let mut tracks = Vec::with_capacity(count as usize);
        for index in 0..count {
            let prefix = format!("track-list/{index}");
            let Some(kind) = self.get_property_string(&format!("{prefix}/type"))? else {
                continue;
            };
            let Some(id) = self.get_property_i64(&format!("{prefix}/id"))? else {
                continue;
            };
            tracks.push(MediaTrack {
                id,
                kind: parse_track_kind(kind),
                title: self.get_property_string(&format!("{prefix}/title"))?,
                language: self.get_property_string(&format!("{prefix}/lang"))?,
                codec: self.get_property_string(&format!("{prefix}/codec"))?,
                selected: self
                    .get_property_flag(&format!("{prefix}/selected"))?
                    .unwrap_or(false),
                default: self
                    .get_property_flag(&format!("{prefix}/default"))?
                    .unwrap_or(false),
                forced: self
                    .get_property_flag(&format!("{prefix}/forced"))?
                    .unwrap_or(false),
                external: self
                    .get_property_flag(&format!("{prefix}/external"))?
                    .unwrap_or(false),
            });
        }
        self.state.tracks = tracks;
        Ok(())
    }

    fn set_optional_track(&mut self, property: &str, track_id: Option<i64>) -> PlayerResult<()> {
        match track_id {
            Some(id) if id >= 0 => self.set_property_i64(property, id),
            Some(id) => Err(PlayerError::InvalidValue {
                name: "track id",
                value: id.to_string(),
            }),
            None => self.set_property_string(property, "no"),
        }
    }

    fn set_property_raw(
        &mut self,
        name: &str,
        format: c_int,
        data: *mut c_void,
    ) -> PlayerResult<()> {
        let name = ffi_string(name, "property name")?;
        unsafe {
            check(
                (self.set_property)(self.handle, name.as_ptr(), format, data),
                self.error_string,
                format!("setting property {}", name.to_string_lossy()),
            )
        }
    }

    fn command_owned(&mut self, values: &[String]) -> PlayerResult<()> {
        if values.is_empty() {
            return Err(PlayerError::InvalidValue {
                name: "command",
                value: "empty argument list".to_owned(),
            });
        }
        let strings = values
            .iter()
            .map(|value| ffi_string(value, "command argument"))
            .collect::<PlayerResult<Vec<_>>>()?;
        let mut args = strings
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        args.push(std::ptr::null());
        unsafe {
            check(
                (self.command_fn)(self.handle, args.as_ptr()),
                self.error_string,
                format!("command {}", values[0]),
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
            if !self.handle.is_null() {
                (self.destroy)(self.handle)
            }
        }
    }
}

fn parse_track_kind(value: String) -> TrackKind {
    match value.as_str() {
        "video" => TrackKind::Video,
        "audio" => TrackKind::Audio,
        "sub" => TrackKind::Subtitle,
        _ => TrackKind::Unknown(value),
    }
}

fn observed_property(userdata: u64, name: *const c_char) -> Option<ObservedProperty> {
    if let Some(spec) = OBSERVED_PROPERTIES.iter().find(|spec| spec.id == userdata) {
        return Some(spec.property);
    }
    if name.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();
    OBSERVED_PROPERTIES
        .iter()
        .find(|spec| spec.name.as_bytes() == name)
        .map(|spec| spec.property)
}

fn validate_finite_range(
    name: &'static str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> PlayerResult<()> {
    if value.is_finite() && (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(invalid_value(name, value))
    }
}

fn invalid_value(name: &'static str, value: f64) -> PlayerError {
    PlayerError::InvalidValue {
        name,
        value: value.to_string(),
    }
}

fn optional_property<T>(result: PlayerResult<T>) -> PlayerResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.is_property_unavailable() => Ok(None),
        Err(error) => Err(error),
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> PlayerResult<T> {
    library
        .get(name)
        .map(|value| *value)
        .map_err(|error| PlayerError::InvalidLibrary(error.to_string()))
}

fn literal(value: &str) -> CString {
    CString::new(value).expect("hard-coded libmpv string cannot contain NUL")
}

fn ffi_string(value: &str, context: &'static str) -> PlayerResult<CString> {
    CString::new(value).map_err(|_| PlayerError::InteriorNul { context })
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

fn load_library() -> PlayerResult<Library> {
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
    Err(PlayerError::LibraryUnavailable(errors.join("; ")))
}

unsafe fn check(
    code: c_int,
    error_string: ErrorString,
    operation: impl Into<String>,
) -> PlayerResult<()> {
    if code >= 0 {
        Ok(())
    } else {
        Err(PlayerError::Mpv {
            operation: operation.into(),
            code,
            message: error_message(error_string, code),
        })
    }
}

unsafe fn error_message(error_string: ErrorString, code: c_int) -> String {
    let message = error_string(code);
    if message.is_null() {
        format!("unknown error ({code})")
    } else {
        CStr::from_ptr(message).to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_bounded_and_requires_a_duration() {
        let mut state = PlayerState {
            position_seconds: Some(25.0),
            duration_seconds: Some(100.0),
            ..PlayerState::default()
        };
        assert_eq!(state.progress(), Some(0.25));
        state.position_seconds = Some(125.0);
        assert_eq!(state.progress(), Some(1.0));
        state.duration_seconds = None;
        assert_eq!(state.progress(), None);
    }

    #[test]
    fn active_phase_prioritizes_buffering_then_pause() {
        let mut state = PlayerState {
            has_media: true,
            phase: PlaybackPhase::Playing,
            paused: true,
            ..PlayerState::default()
        };
        state.update_active_phase();
        assert_eq!(state.phase, PlaybackPhase::Paused);
        state.paused_for_cache = true;
        state.update_active_phase();
        assert_eq!(state.phase, PlaybackPhase::Buffering);
        state.paused_for_cache = false;
        state.paused = false;
        state.update_active_phase();
        assert_eq!(state.phase, PlaybackPhase::Playing);
    }

    #[test]
    fn new_load_resets_stale_media_values() {
        let mut state = PlayerState {
            phase: PlaybackPhase::Ended,
            has_media: true,
            position_seconds: Some(90.0),
            duration_seconds: Some(90.0),
            eof_reached: true,
            media_title: Some("Old title".into()),
            last_error: Some("old error".into()),
            ..PlayerState::default()
        };
        state.start_loading();
        assert_eq!(state.phase, PlaybackPhase::Loading);
        assert_eq!(state.position_seconds, None);
        assert_eq!(state.duration_seconds, None);
        assert!(!state.eof_reached);
        assert_eq!(state.media_title, None);
        assert_eq!(state.last_error, None);
    }

    #[test]
    fn validates_control_ranges_and_non_finite_values() {
        assert!(validate_finite_range("volume", 0.0, 0.0, 130.0).is_ok());
        assert!(validate_finite_range("volume", 130.0, 0.0, 130.0).is_ok());
        assert!(validate_finite_range("volume", 131.0, 0.0, 130.0).is_err());
        assert!(validate_finite_range("volume", f64::NAN, 0.0, 130.0).is_err());
    }

    #[test]
    fn parses_known_and_future_track_kinds() {
        assert_eq!(parse_track_kind("video".into()), TrackKind::Video);
        assert_eq!(parse_track_kind("audio".into()), TrackKind::Audio);
        assert_eq!(parse_track_kind("sub".into()), TrackKind::Subtitle);
        assert_eq!(
            parse_track_kind("data".into()),
            TrackKind::Unknown("data".into())
        );
    }

    #[test]
    fn rejects_nul_bytes_before_crossing_ffi() {
        assert!(matches!(
            ffi_string("bad\0value", "test"),
            Err(PlayerError::InteriorNul { context: "test" })
        ));
    }

    #[test]
    fn maps_end_reasons_without_losing_unknown_values() {
        assert_eq!(EndReason::from(0), EndReason::EndOfFile);
        assert_eq!(EndReason::from(4), EndReason::Error);
        assert_eq!(EndReason::from(99), EndReason::Unknown(99));
    }
}
