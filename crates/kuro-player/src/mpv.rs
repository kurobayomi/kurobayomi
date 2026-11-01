//! Safe wrapper over libmpv2 (area 45 p1-rust-design section 3.5). One handle per player window,
//! used only on the GTK main thread. The GL render context is created here from a caller-supplied
//! `get_proc_address`; the `gtk::GLArea` wiring lives in the GTK crate, not in this library.

use std::ffi::c_void;

use libmpv2::Format;
use libmpv2::events::{Event, PropertyData};
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};

use crate::observer::{MpvCommands, MpvEvent, PropFormat, RawValue, format_of, property_event};
use crate::options::MpvSetting;
use crate::tracks::TrackValue;

/// Video output required by the render API. `startup_settings` still carries the preferred
/// `gpu`/`gpu-next` value (Kotlin contract); `create` overrides it with this one so the frames
/// reach the GL render context instead of an mpv-owned window.
pub const RENDER_VO: &str = "libmpv";

/// `MPV_ERROR_OPTION_NOT_FOUND` from `mpv/client.h`.
const MPV_ERROR_OPTION_NOT_FOUND: i32 = -5;

/// Resolves a GL function name. Passed as a plain function pointer so no state is captured.
pub type GetProcAddress = fn(&str) -> *mut c_void;

#[derive(Debug, thiserror::Error)]
pub enum MpvError {
    #[error("libmpv: {0}")]
    Mpv(#[from] libmpv2::Error),
    #[error("render context already created")]
    RenderExists,
    #[error("render context not created")]
    NoRenderContext,
    #[error("empty mpv command")]
    EmptyCommand,
}

/// String property writes, so code that sends them (metadata options, fonts, filters) can be tested
/// with a recording fake. `MpvHandle` is the real sink.
pub trait PropertySink {
    fn set_str(&self, name: &str, value: &str) -> Result<(), MpvError>;

    /// An integer property (`MPVLib.setPropertyInt`). A sink with no typed write sends the text.
    fn set_i64(&self, name: &str, value: i64) -> Result<(), MpvError> {
        self.set_str(name, &value.to_string())
    }

    /// A double property (`MPVLib.setPropertyDouble`). A sink with no typed write sends the text.
    fn set_f64(&self, name: &str, value: f64) -> Result<(), MpvError> {
        self.set_str(name, &value.to_string())
    }
}

impl PropertySink for MpvHandle {
    fn set_str(&self, name: &str, value: &str) -> Result<(), MpvError> {
        self.set_property_str(name, value)
    }

    fn set_i64(&self, name: &str, value: i64) -> Result<(), MpvError> {
        self.set_property_i64(name, value)
    }

    fn set_f64(&self, name: &str, value: f64) -> Result<(), MpvError> {
        self.set_property_f64(name, value)
    }
}

/// String property reads, so the `metadata` read after `FILE_LOADED` can be tested with a fake.
pub trait PropertySource {
    fn get_str(&self, name: &str) -> Option<String>;
}

impl PropertySource for MpvHandle {
    fn get_str(&self, name: &str) -> Option<String> {
        self.get_string(name)
    }
}

/// Map from the observation format class to the libmpv2 format. libmpv2 has no `NONE` format and
/// drops `NONE` notifications, so `None` observations use `String`; the value is then ignored.
fn libmpv_format(format: PropFormat) -> Format {
    match format {
        PropFormat::None | PropFormat::Str => Format::String,
        PropFormat::Int => Format::Int64,
        PropFormat::Flag => Format::Flag,
        PropFormat::Double => Format::Double,
    }
}

/// `tls-verify` is on and `tls-ca-file` points at the bundled `cacert.pem`, which nothing had installed: every https
/// stream then failed with "Peer certificate failed verification". Installs the bundled assets next to it; when the
/// file is still missing (read-only folder), the option is dropped so mpv uses the system certificate store.
fn ca_file_missing(settings: &[MpvSetting]) -> bool {
    let Some(setting) = settings.iter().find(|s| s.name == "tls-ca-file") else {
        return false;
    };
    let path = std::path::Path::new(&setting.value);
    if path.is_file() {
        return false;
    }
    if let Some(dir) = path.parent() {
        crate::assets::install_assets(dir);
    }
    if path.is_file() {
        return false;
    }
    tracing::warn!(file = %setting.value, "the TLS certificate file is missing; mpv uses the system certificates");
    true
}

pub struct MpvHandle {
    // Declared before `inner`, so the render context is freed before `mpv_destroy` runs.
    render: Option<RenderContext<'static>>,
    inner: Box<libmpv2::Mpv>,
    next_observe_id: u64,
}

impl MpvHandle {
    /// Creates the core with every setting applied before initialisation, in order.
    pub fn create(settings: &[MpvSetting]) -> Result<Self, MpvError> {
        let drop_ca_file = ca_file_missing(settings);
        let inner = libmpv2::Mpv::with_initializer(|init| {
            for setting in settings {
                if drop_ca_file && setting.name == "tls-ca-file" {
                    continue;
                }
                // An older libmpv (Ubuntu 24.04 ships 0.37) does not know every option; skip those
                // instead of failing to start the player.
                match init.set_option(setting.name, setting.value.as_str()) {
                    Err(libmpv2::Error::Raw(MPV_ERROR_OPTION_NOT_FOUND)) => {
                        tracing::warn!(option = setting.name, "this libmpv does not know the option; skipped");
                    }
                    other => other?,
                }
            }
            // Set last so it wins over the preference table (see `RENDER_VO`).
            init.set_option("vo", RENDER_VO)?;
            Ok(())
        })?;
        Ok(Self {
            render: None,
            inner: Box::new(inner),
            next_observe_id: 1,
        })
    }

    pub fn set_property_str(&self, name: &str, value: &str) -> Result<(), MpvError> {
        Ok(self.inner.set_property(name, value)?)
    }

    pub fn set_property_i64(&self, name: &str, value: i64) -> Result<(), MpvError> {
        Ok(self.inner.set_property(name, value)?)
    }

    pub fn set_property_f64(&self, name: &str, value: f64) -> Result<(), MpvError> {
        Ok(self.inner.set_property(name, value)?)
    }

    pub fn set_property_flag(&self, name: &str, value: bool) -> Result<(), MpvError> {
        Ok(self.inner.set_property(name, value)?)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        self.inner.get_property(name).ok()
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.inner.get_property(name).ok()
    }

    pub fn get_flag(&self, name: &str) -> Option<bool> {
        self.inner.get_property(name).ok()
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        self.inner.get_property(name).ok()
    }

    /// Selects a track (`aid`, `sid`, `secondary-sid`). `TRACK_NONE` disables it.
    pub fn set_track(&self, property: &str, id: i64) -> Result<(), MpvError> {
        match TrackValue::for_id(id) {
            TrackValue::No => self.set_property_str(property, "no"),
            TrackValue::Id(id) => self.set_property_i64(property, id),
        }
    }

    /// Runs an mpv command; `args[0]` is the command name.
    pub fn command(&self, args: &[&str]) -> Result<(), MpvError> {
        let (name, rest) = args.split_first().ok_or(MpvError::EmptyCommand)?;
        Ok(self.inner.command(name, rest)?)
    }

    /// Runs `commands` one after another on a worker thread, through a client handle of the same player core, so a slow
    /// one (a side audio or subtitle track is fetched over the network, 25 of them for some sources) never blocks the
    /// thread that asks, which is the GTK thread. Failures are logged. The handle is only destroyed, not terminated,
    /// so a close of the player in the meantime just makes the remaining commands fail.
    pub fn run_commands_in_background(&self, commands: Vec<Vec<String>>) {
        self.run_commands_in_background_with(commands, |command| command);
    }

    /// Like [`Self::run_commands_in_background`], with each command passed through `prepare` on the worker thread first
    /// (the app swaps a side subtitle's address for its cached copy there, fetching it if needed).
    pub fn run_commands_in_background_with(&self, commands: Vec<Vec<String>>, prepare: impl Fn(Vec<String>) -> Vec<String> + Send + 'static) {
        let client = match self.inner.create_client(None) {
            Ok(client) => client,
            Err(e) => return tracing::warn!(error = %e, "mpv: no client handle for the background commands"),
        };
        std::thread::spawn(move || {
            for command in commands {
                let command = prepare(command);
                let Some((name, rest)) = command.split_first() else { continue };
                let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
                if let Err(e) = client.command(name, &rest) {
                    tracing::warn!(error = %e, command = %name, "mpv: a background command failed");
                }
            }
        });
    }

    /// Registers the wakeup callback. The GTK side uses it to schedule `drain_events` on the
    /// main loop; the callback may run on any thread, so it should only signal.
    pub fn set_wakeup_callback<F: Fn() + Send + 'static>(&mut self, callback: F) {
        self.inner.set_wakeup_callback(callback);
    }

    /// Pulls every queued event and converts it. Main thread only.
    pub fn drain_events(&mut self) -> Vec<MpvEvent> {
        let mut out = Vec::new();
        while let Some(next) = self.inner.wait_event(0.0) {
            match next {
                Ok(event) => out.extend(convert(event)),
                Err(e) => tracing::warn!(error = %e, "mpv event decode failed"),
            }
        }
        out
    }

    /// Creates the OpenGL render context. Must run on the thread that owns the GL context, after
    /// the GL area is realized. Fails if one already exists.
    pub fn create_render_context(
        &mut self,
        get_proc_address: GetProcAddress,
    ) -> Result<(), MpvError> {
        if self.render.is_some() {
            return Err(MpvError::RenderExists);
        }
        let params = [
            RenderParam::ApiType(RenderParamApiType::OpenGl),
            RenderParam::InitParams(OpenGLInitParams {
                get_proc_address: |resolve: &GetProcAddress, name: &str| resolve(name),
                ctx: get_proc_address,
            }),
        ];
        let context = self.inner.create_render_context(params)?;
        // SAFETY: the context only holds a raw mpv_render_context pointer and a marker for the
        // borrow of `inner`. `inner` is boxed, so its address is stable, and the field order makes
        // `render` drop before `inner`. The `'static` lifetime only records that order.
        let context: RenderContext<'static> = unsafe { std::mem::transmute(context) };
        self.render = Some(context);
        Ok(())
    }

    /// Frees the render context. The GL area calls this on unrealize, on the thread that owns the
    /// GL context, so a later realize can create a new one. Returns whether one existed. Dropping
    /// the handle frees it too, but only this runs while the window stays open.
    pub fn destroy_render_context(&mut self) -> bool {
        self.render.take().is_some()
    }

    /// Draws one frame into `fbo`, with the GL origin at the bottom-left (`flip` = true).
    pub fn render(&self, fbo: i32, width: i32, height: i32) -> Result<(), MpvError> {
        let context = self.render.as_ref().ok_or(MpvError::NoRenderContext)?;
        Ok(context.render::<()>(fbo, width, height, true)?)
    }

    /// Registers the callback mpv calls when a new frame is wanted (the GTK side queues a redraw).
    pub fn set_render_update_callback<F: Fn() + Send + 'static>(
        &mut self,
        callback: F,
    ) -> Result<(), MpvError> {
        let context = self.render.as_mut().ok_or(MpvError::NoRenderContext)?;
        context.set_update_callback(callback);
        Ok(())
    }
}

impl MpvCommands for MpvHandle {
    fn observe(&mut self, name: &str, format: PropFormat) -> Result<(), MpvError> {
        let id = self.next_observe_id;
        self.next_observe_id += 1;
        self.inner
            .observe_property(name, libmpv_format(format), id)?;
        Ok(())
    }
}

/// Converts one libmpv event. Events the player does not use map to `None`.
fn convert(event: Event<'_>) -> Option<MpvEvent> {
    match event {
        Event::FileLoaded => Some(MpvEvent::FileLoaded),
        Event::Seek => Some(MpvEvent::Seek),
        Event::PlaybackRestart => Some(MpvEvent::PlaybackRestart),
        Event::EndFile(reason) => {
            // mpv_end_file_reason: 0 = EOF, 4 = ERROR. The error text is not exposed by libmpv2.
            let reason: u32 = reason;
            Some(MpvEvent::EndFile {
                eof_reached: reason == 0,
                error: (reason == 4).then(String::new),
            })
        }
        Event::PropertyChange { name, change, .. } => {
            let format = format_of(name)?;
            property_event(name, format, raw_value(change)?)
        }
        _ => None,
    }
}

fn raw_value(data: PropertyData<'_>) -> Option<RawValue> {
    Some(match data {
        PropertyData::Str(s) | PropertyData::OsdStr(s) => RawValue::Str(s.to_owned()),
        PropertyData::Flag(b) => RawValue::Flag(b),
        PropertyData::Int64(i) => RawValue::Int(i),
        PropertyData::Double(d) => RawValue::Double(d),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{PlayerPrefs, startup_settings};
    use std::path::Path;

    /// Headless handle whose data directory is `data_dir`: no window is opened because no render
    /// context is created.
    fn headless_in(data_dir: &Path) -> MpvHandle {
        let prefs = PlayerPrefs {
            data_dir: data_dir.to_owned(),
            ..PlayerPrefs::default()
        };
        let mut settings = startup_settings(&prefs);
        settings.push(MpvSetting {
            name: "ao",
            value: "null".to_owned(),
        });
        MpvHandle::create(&settings).expect("libmpv must initialise headless")
    }

    fn headless() -> MpvHandle {
        let dir = tempfile::tempdir().expect("temp data dir");
        headless_in(dir.path())
    }

    #[test]
    fn a_missing_ca_file_is_installed_or_dropped_so_https_can_verify() {
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("mpv").join("cacert.pem");
        let mut settings = startup_settings(&PlayerPrefs::default());
        settings.retain(|s| s.name != "tls-ca-file");
        settings.push(MpvSetting { name: "ao", value: "null".to_owned() });
        settings.push(MpvSetting { name: "tls-ca-file", value: ca.to_string_lossy().into_owned() });
        assert!(MpvHandle::create(&settings).is_ok());
        assert!(ca.is_file(), "the bundled certificates are installed where the option points");
        // a folder that cannot be written: the option is dropped and mpv still starts
        let blocked = dir.path().join("file");
        std::fs::write(&blocked, b"x").unwrap();
        settings.retain(|s| s.name != "tls-ca-file");
        settings.push(MpvSetting { name: "tls-ca-file", value: blocked.join("mpv").join("cacert.pem").to_string_lossy().into_owned() });
        assert!(MpvHandle::create(&settings).is_ok());
    }

    #[test]
    #[ignore = "slow: run on demand"]
    fn background_commands_never_block_the_caller() {
        let handle = headless();
        let start = std::time::Instant::now();
        // three side tracks that cannot load (nothing is playing): each would block a synchronous call
        handle.run_commands_in_background(vec![
            vec!["audio-add".into(), "https://invalid.test/a.m3u8".into(), "select".into(), "x".into()],
            vec!["sub-add".into(), "https://invalid.test/s.vtt".into(), "select".into(), "x".into()],
            Vec::new(),
        ]);
        assert!(start.elapsed() < std::time::Duration::from_millis(500), "returns at once");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(handle.get_flag("idle-active"), Some(true), "the core is still alive and idle");
    }

    #[test]
    fn an_option_this_libmpv_does_not_know_is_skipped() {
        let mut settings = startup_settings(&PlayerPrefs::default());
        settings.push(MpvSetting {
            name: "ao",
            value: "null".to_owned(),
        });
        settings.push(MpvSetting {
            name: "option-from-a-future-mpv",
            value: "yes".to_owned(),
        });
        assert!(MpvHandle::create(&settings).is_ok());
    }

    #[test]
    fn mpv_conf_in_config_dir_is_applied() {
        let data = tempfile::tempdir().unwrap();
        let mpv_dir = data.path().join("mpv");
        std::fs::create_dir_all(&mpv_dir).unwrap();
        std::fs::write(mpv_dir.join("mpv.conf"), "volume=37\n").unwrap();
        let handle = headless_in(data.path());
        assert_eq!(handle.get_flag("config"), Some(true));
        assert_eq!(handle.get_string("config-dir").as_deref(), mpv_dir.to_str());
        assert_eq!(handle.get_f64("volume"), Some(37.0));
    }

    #[test]
    fn without_mpv_conf_volume_keeps_mpv_default() {
        let handle = headless();
        assert_eq!(handle.get_f64("volume"), Some(100.0));
    }

    #[test]
    fn create_starts_paused_and_forces_render_vo() {
        let handle = headless();
        assert_eq!(handle.get_flag("pause"), Some(true));
        assert_eq!(handle.get_string("vo").as_deref(), Some(RENDER_VO));
    }

    #[test]
    fn empty_command_is_rejected() {
        let handle = headless();
        assert!(matches!(handle.command(&[]), Err(MpvError::EmptyCommand)));
    }

    #[test]
    fn render_calls_fail_without_context() {
        let handle = headless();
        assert!(matches!(
            handle.render(0, 1, 1),
            Err(MpvError::NoRenderContext)
        ));
    }

    #[test]
    fn destroying_the_render_context_is_safe_without_one_and_leaves_render_unavailable() {
        let mut handle = headless();
        assert!(!handle.destroy_render_context());
        assert!(!handle.destroy_render_context());
        assert!(matches!(
            handle.render(0, 1, 1),
            Err(MpvError::NoRenderContext)
        ));
    }

    #[test]
    fn disabled_track_is_written_as_no() {
        let handle = headless();
        handle.set_track("aid", -1).unwrap();
        assert_eq!(handle.get_string("aid").as_deref(), Some("no"));
    }

    #[test]
    fn observe_registers_properties() {
        let mut handle = headless();
        crate::observer::observe_all(&mut handle).expect("observe all");
        assert_eq!(handle.next_observe_id, 20);
    }
}
