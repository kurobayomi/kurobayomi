//! mpv events and property observation (`observedProps` in
//! `AniyomiMPVView.kt:210-252`, `PlayerObserver.kt`). The conversion from libmpv raw values is
//! pure here; `mpv.rs` feeds it. Events cross to the GTK main loop through `event_channel`.

use crate::mpv::MpvError;

/// Format class a property is observed with. Mirrors the Kotlin `MPV_FORMAT_*` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropFormat {
    None,
    Int,
    Flag,
    Str,
    Double,
}

/// The observed properties, in source order (`AniyomiMPVView.kt:210-252`). The `user-data/*` bridge is cut.
pub const OBSERVED: [(&str, PropFormat); 19] = [
    ("chapter", PropFormat::Int),
    ("chapter-list", PropFormat::None),
    ("track-list", PropFormat::None),
    ("time-pos", PropFormat::Int),
    ("demuxer-cache-time", PropFormat::Int),
    ("duration", PropFormat::Int),
    ("volume", PropFormat::Int),
    ("volume-max", PropFormat::Int),
    ("sid", PropFormat::Str),
    ("secondary-sid", PropFormat::Str),
    ("aid", PropFormat::Str),
    ("speed", PropFormat::Double),
    ("video-params/aspect", PropFormat::Double),
    ("hwdec-current", PropFormat::Str),
    ("hwdec", PropFormat::Str),
    ("pause", PropFormat::Flag),
    ("paused-for-cache", PropFormat::Flag),
    ("seeking", PropFormat::Flag),
    ("eof-reached", PropFormat::Flag),
];

/// Format class of an observed property, or `None` for names outside the table.
pub fn format_of(name: &str) -> Option<PropFormat> {
    OBSERVED.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
}

/// A raw property value as libmpv delivers it.
#[derive(Clone, Debug, PartialEq)]
pub enum RawValue {
    Int(i64),
    Flag(bool),
    Str(String),
    Double(f64),
}

/// Typed events the GTK side consumes. Property names are owned so events can cross threads.
#[derive(Clone, Debug, PartialEq)]
pub enum MpvEvent {
    FileLoaded,
    Seek,
    PlaybackRestart,
    /// `error` is `Some("")` when mpv reports an error without a message. libmpv2 6.0.0 does
    /// not expose the message, so the toast falls back to `Error: File ended`.
    EndFile {
        eof_reached: bool,
        error: Option<String>,
    },
    Property(String),
    PropertyInt(String, i64),
    PropertyFlag(String, bool),
    PropertyStr(String, String),
    PropertyDouble(String, f64),
}

/// Turns one property change into a typed event. A value whose type does not match the
/// observed format class is dropped (`None`), as Kotlin's overloads do not fire for it.
pub fn property_event(name: &str, format: PropFormat, raw: RawValue) -> Option<MpvEvent> {
    let name = name.to_owned();
    match (format, raw) {
        (PropFormat::None, _) => Some(MpvEvent::Property(name)),
        (PropFormat::Int, RawValue::Int(i)) => Some(MpvEvent::PropertyInt(name, i)),
        (PropFormat::Int, RawValue::Double(d)) => Some(MpvEvent::PropertyInt(name, d as i64)),
        (PropFormat::Flag, RawValue::Flag(b)) => Some(MpvEvent::PropertyFlag(name, b)),
        (PropFormat::Str, RawValue::Str(s)) => Some(MpvEvent::PropertyStr(name, s)),
        (PropFormat::Double, RawValue::Double(d)) => Some(MpvEvent::PropertyDouble(name, d)),
        (PropFormat::Double, RawValue::Int(i)) => Some(MpvEvent::PropertyDouble(name, i as f64)),
        _ => None,
    }
}

/// Passes a `time-pos` value on only when the whole second changes (T-45b-073). The caller sends
/// the position events to the GTK loop through this gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WholeSecondGate {
    last: Option<i64>,
}

impl WholeSecondGate {
    /// `true` for the first value and for each new whole second. Repeats are dropped.
    pub fn accept(&mut self, whole_second: i64) -> bool {
        if self.last == Some(whole_second) {
            return false;
        }
        self.last = Some(whole_second);
        true
    }
}

/// The subset of the mpv handle that observation needs. `MpvHandle` implements it; tests use a
/// fake that records calls.
pub trait MpvCommands {
    fn observe(&mut self, name: &str, format: PropFormat) -> Result<(), MpvError>;
}

/// Registers every entry of `OBSERVED` (behaviour doc 2.9).
pub fn observe_all(target: &mut impl MpvCommands) -> Result<(), MpvError> {
    for (name, format) in OBSERVED {
        target.observe(name, format)?;
    }
    Ok(())
}

/// The level of a forwarded mpv log line in the Rust logger (T-45a-008). `fatal` and `error` are
/// errors, `warn` is a warning, `info` is info, and every other level (`v`, `debug`, `trace`) is
/// debug.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

pub fn log_level(mpv_level: &str) -> LogLevel {
    match mpv_level {
        "fatal" | "error" => LogLevel::Error,
        "warn" => LogLevel::Warn,
        "info" => LogLevel::Info,
        _ => LogLevel::Debug,
    }
}

/// Forwards one mpv log line to the Rust logger. The target is `mpv`, and the mpv module prefix is
/// the `prefix` field (a tracing target must be a constant, so `mpv/<prefix>` cannot be built).
pub fn forward_log(prefix: &str, mpv_level: &str, text: &str) {
    let text = text.trim_end();
    match log_level(mpv_level) {
        LogLevel::Error => tracing::error!(target: "mpv", prefix, "{text}"),
        LogLevel::Warn => tracing::warn!(target: "mpv", prefix, "{text}"),
        LogLevel::Info => tracing::info!(target: "mpv", prefix, "{text}"),
        LogLevel::Debug => tracing::debug!(target: "mpv", prefix, "{text}"),
    }
}

/// Error text for end-file events and the last HTTP error from the mpv log (T-45a-008 / 009).
#[derive(Debug, Default)]
pub struct ErrorReporter {
    last_http_error: Option<String>,
}

/// Toast duration for playback errors, in seconds (Android `Toast.LENGTH_LONG` on GTK).
pub const ERROR_TOAST_SECS: u32 = 5;

impl ErrorReporter {
    /// Stores any log line that contains `HTTP error`, replacing the previous one.
    pub fn log_line(&mut self, text: &str) {
        if text.contains("HTTP error") {
            self.last_http_error = Some(text.to_owned());
        }
    }

    /// `PlayerObserver.efEvent`: an empty or missing error becomes `Error: File ended`. A stored
    /// HTTP error is appended as `: <text>` and cleared.
    pub fn end_file_message(&mut self, error: Option<&str>) -> String {
        let mut message = match error {
            Some(e) if !e.is_empty() => e.to_owned(),
            _ => "Error: File ended".to_owned(),
        };
        if let Some(http) = self.last_http_error.take() {
            if !http.is_empty() {
                message.push_str(": ");
                message.push_str(&http);
            }
        }
        message
    }
}

/// Unbounded channel from the mpv event pump to the GTK main loop. Events are never dropped
/// here; the consumer decides what to ignore while the window is exiting (T-45a-039).
pub fn event_channel() -> (
    async_channel::Sender<MpvEvent>,
    async_channel::Receiver<MpvEvent>,
) {
    async_channel::unbounded()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_events_fire_only_on_a_whole_second_change() {
        let mut gate = WholeSecondGate::default();
        let fired: Vec<bool> = [3, 3, 3, 4, 4, 5]
            .into_iter()
            .map(|s| gate.accept(s))
            .collect();
        assert_eq!(fired, vec![true, false, false, true, false, true]);
    }

    #[derive(Default)]
    struct FakeMpv {
        observed: Vec<(String, PropFormat)>,
    }

    impl MpvCommands for FakeMpv {
        fn observe(&mut self, name: &str, format: PropFormat) -> Result<(), MpvError> {
            self.observed.push((name.to_owned(), format));
            Ok(())
        }
    }

    #[test]
    fn observes_all_19_with_their_formats() {
        let mut fake = FakeMpv::default();
        observe_all(&mut fake).unwrap();
        assert_eq!(fake.observed.len(), 19);
        assert_eq!(fake.observed[0], ("chapter".to_owned(), PropFormat::Int));
        assert!(
            fake.observed
                .iter()
                .any(|(n, f)| n == "pause" && *f == PropFormat::Flag)
        );
        assert!(
            fake.observed
                .iter()
                .any(|(n, f)| n == "speed" && *f == PropFormat::Double)
        );
        assert!(
            fake.observed
                .iter()
                .any(|(n, f)| n == "track-list" && *f == PropFormat::None)
        );
    }

    #[test]
    fn each_format_class_yields_its_event() {
        assert_eq!(
            property_event("pause", PropFormat::Flag, RawValue::Flag(true)),
            Some(MpvEvent::PropertyFlag("pause".into(), true))
        );
        assert_eq!(
            property_event("time-pos", PropFormat::Int, RawValue::Int(42)),
            Some(MpvEvent::PropertyInt("time-pos".into(), 42))
        );
        assert_eq!(
            property_event("speed", PropFormat::Double, RawValue::Double(1.5)),
            Some(MpvEvent::PropertyDouble("speed".into(), 1.5))
        );
        assert_eq!(
            property_event("hwdec", PropFormat::Str, RawValue::Str("vaapi".into())),
            Some(MpvEvent::PropertyStr("hwdec".into(), "vaapi".into()))
        );
        assert_eq!(
            property_event("track-list", PropFormat::None, RawValue::Str(String::new())),
            Some(MpvEvent::Property("track-list".into()))
        );
    }

    #[test]
    fn mismatched_value_type_is_dropped() {
        assert_eq!(
            property_event("pause", PropFormat::Flag, RawValue::Int(1)),
            None
        );
    }

    #[test]
    fn end_file_without_message_is_file_ended() {
        let mut r = ErrorReporter::default();
        assert_eq!(r.end_file_message(None), "Error: File ended");
        assert_eq!(r.end_file_message(Some("")), "Error: File ended");
    }

    #[test]
    fn stored_http_error_is_appended_once() {
        let mut r = ErrorReporter::default();
        r.log_line("unrelated line");
        r.log_line("cplayer: HTTP error 403 Forbidden");
        assert_eq!(
            r.end_file_message(None),
            "Error: File ended: cplayer: HTTP error 403 Forbidden"
        );
        assert_eq!(r.end_file_message(None), "Error: File ended");
    }

    #[test]
    fn events_cross_the_channel() {
        let (tx, rx) = event_channel();
        let handle = std::thread::spawn(move || {
            tx.send_blocking(MpvEvent::FileLoaded).unwrap();
            tx.send_blocking(MpvEvent::Seek).unwrap();
        });
        handle.join().unwrap();
        assert_eq!(rx.try_recv().unwrap(), MpvEvent::FileLoaded);
        assert_eq!(rx.try_recv().unwrap(), MpvEvent::Seek);
    }
}

#[cfg(test)]
mod log_level_tests {
    use super::*;

    #[test]
    fn t_fatal_and_error_are_errors_warn_is_warn_info_is_info_rest_is_debug() {
        assert_eq!(log_level("fatal"), LogLevel::Error);
        assert_eq!(log_level("error"), LogLevel::Error);
        assert_eq!(log_level("warn"), LogLevel::Warn);
        assert_eq!(log_level("info"), LogLevel::Info);
        assert_eq!(log_level("v"), LogLevel::Debug);
        assert_eq!(log_level("debug"), LogLevel::Debug);
        assert_eq!(log_level("trace"), LogLevel::Debug);
        assert_eq!(log_level("unknown"), LogLevel::Debug);
    }

    #[test]
    fn forwarding_a_line_does_not_panic_at_any_level() {
        forward_log("cplayer", "fatal", "boom\n");
        forward_log("ffmpeg", "info", "ready");
        forward_log("ffmpeg", "v", "chatter");
    }
}
