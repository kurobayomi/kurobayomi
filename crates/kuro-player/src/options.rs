//! Pure mapping from player preferences to the startup mpv option table
//! (`AniyomiMPVView.kt:114-156`, `setupSubtitlesOptions`, `setupAudioOptions`, area 45 data doc
//! section 4 and behaviour doc 2.2). No libmpv and no I/O here: `startup_settings` returns the
//! ordered list that `MpvHandle::create` applies before `mpv_initialize`.

use std::path::{Path, PathBuf};

use crate::enums::{
    AudioChannels, Debanding, Decoder, GpuKind, SubtitleJustification, SubtitlesBorderStyle,
};

/// One startup setting. Every entry is applied before initialisation with `set_option`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MpvSetting {
    pub name: &'static str,
    pub value: String,
}

/// Demuxer cache size on Linux: 64 MiB for both forward and back cache (T-45a-005).
pub const CACHE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct SubtitlePrefs {
    /// `pref_subtitle_lang`: the subtitle languages picked first, comma separated (mpv `slang`).
    pub lang: String,
    pub delay_ms: i32,
    pub speed: f32,
    pub secondary_delay_ms: i32,
    pub font: String,
    pub font_size: i32,
    pub bold: bool,
    pub italic: bool,
    pub justify: SubtitleJustification,
    pub text_color_argb: i32,
    pub back_color_argb: i32,
    pub border_color_argb: i32,
    pub border_size: i32,
    pub border_style: SubtitlesBorderStyle,
    pub shadow_offset: i32,
    pub pos: i32,
    pub scale: f32,
    pub override_ass: bool,
}

impl Default for SubtitlePrefs {
    fn default() -> Self {
        Self {
            lang: String::new(),
            delay_ms: 0,
            speed: 1.0,
            secondary_delay_ms: 0,
            font: "Sans Serif".to_owned(),
            font_size: 55,
            bold: false,
            italic: false,
            justify: SubtitleJustification::Auto,
            text_color_argb: -1,
            back_color_argb: 0,
            border_color_argb: -16_777_216,
            border_size: 3,
            border_style: SubtitlesBorderStyle::OutlineAndShadow,
            shadow_offset: 0,
            pos: 100,
            scale: 1.0,
            override_ass: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioPrefs {
    pub lang: String,
    pub delay_ms: i32,
    pub pitch_correction: bool,
    pub channels: AudioChannels,
    pub volume_boost_cap: i32,
}

impl Default for AudioPrefs {
    fn default() -> Self {
        Self {
            lang: String::new(),
            delay_ms: 0,
            pitch_correction: true,
            channels: AudioChannels::AutoSafe,
            volume_boost_cap: 30,
        }
    }
}

/// Pure input for `startup_settings`. `data_dir` holds `mpv/`, `screenshot_dir` is the
/// Pictures folder. `hwdec_available` is the candidate list probed from mpv (empty at startup).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerPrefs {
    pub try_hwdec: bool,
    pub hwdec_available: Vec<String>,
    /// The GPU vendor, which picks the Linux `hwdec` names of the decoder chips.
    pub gpu: GpuKind,
    pub gpu_next: bool,
    pub debanding: Debanding,
    pub use_yuv420p: bool,
    pub verbose_logging: bool,
    pub player_speed: f32,
    /// brightness, saturation, contrast, gamma, hue.
    pub filters: [i32; 5],
    pub sub: SubtitlePrefs,
    pub audio: AudioPrefs,
    pub data_dir: PathBuf,
    pub screenshot_dir: PathBuf,
    /// The decoder chosen last time. It is applied at startup instead of `try_hwdec`.
    pub remembered_decoder: Option<Decoder>,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        Self {
            try_hwdec: true,
            hwdec_available: Vec::new(),
            gpu: GpuKind::default(),
            gpu_next: false,
            debanding: Debanding::None,
            use_yuv420p: true,
            verbose_logging: false,
            player_speed: 1.0,
            filters: [0; 5],
            sub: SubtitlePrefs::default(),
            audio: AudioPrefs::default(),
            data_dir: PathBuf::new(),
            screenshot_dir: PathBuf::new(),
            remembered_decoder: None,
        }
    }
}

/// Kotlin `Double.toString()` for values that come from `/ 1000.0` or stored floats: always a
/// fractional part, so `0` prints `0.0`. Rust's shortest round-trip form matches otherwise.
pub(crate) fn kotlin_f64(v: f64) -> String {
    let s = v.to_string();
    if s.contains('.') { s } else { format!("{s}.0") }
}

/// Kotlin `Float.toString()`, see `kotlin_f64`.
pub(crate) fn kotlin_f32(v: f32) -> String {
    let s = v.to_string();
    if s.contains('.') { s } else { format!("{s}.0") }
}

/// Kotlin `Int.toColorHexString()`: `#` plus eight uppercase hex digits of the ARGB value.
pub fn color_hex(argb: i32) -> String {
    format!("#{:08X}", argb as u32)
}

/// Chooses the mpv `hwdec` value from the decoder and the candidates reported by mpv.
/// Copy-back candidates are tried first for `Hw`; `HwPlus` tries zero-copy first. Only what mpv reports is picked, so
/// the list holds the decoders of every system: VA-API and NVDEC on Linux, D3D11 on Windows. When none of the
/// candidates is available the result is `auto` (area 45 rust-design section 6).
pub fn resolve_hwdec(decoder: Decoder, available: &[String]) -> String {
    let candidates: &[&str] = match decoder {
        Decoder::AutoCopy | Decoder::Auto | Decoder::Sw => return decoder.mpv_value().to_owned(),
        Decoder::Hw => &["vaapi-copy", "nvdec-copy", "d3d11va-copy", "auto-copy"],
        Decoder::HwPlus => &["vaapi", "nvdec", "d3d11va", "vaapi-copy", "nvdec-copy", "d3d11va-copy", "auto-copy"],
    };
    candidates
        .iter()
        .find(|c| available.iter().any(|a| a == *c))
        .map_or_else(|| "auto".to_owned(), |c| (*c).to_owned())
}

/// Clamps a video filter value to -100..=100 before it is written (T-45a-032, proposal: the Kotlin
/// passes stored values through unclamped).
pub fn clamp_filter(value: i64) -> i64 {
    value.clamp(-100, 100)
}

/// The inputs of the one `vf` chain (E-26 fix): CPU debanding, the five video filters and
/// `format=yuv420p`. The filters come from `brightness`/`contrast`/`gamma`/`saturation` of
/// ffmpeg's `eq` filter and from `hue`, so a runtime change rewrites the whole chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoChain {
    pub debanding: Debanding,
    pub use_yuv420p: bool,
    /// brightness, saturation, contrast, gamma, hue, as stored (-100..=100).
    pub filters: [i32; 5],
}

impl Default for VideoChain {
    fn default() -> Self {
        Self::from(&PlayerPrefs::default())
    }
}

impl From<&PlayerPrefs> for VideoChain {
    fn from(p: &PlayerPrefs) -> Self {
        Self {
            debanding: p.debanding,
            use_yuv420p: p.use_yuv420p,
            filters: p.filters,
        }
    }
}

/// A number with at most four decimals and no trailing zeros.
fn trimmed(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

impl VideoChain {
    /// The comma-joined `vf` value, or empty when nothing is enabled. A filter at 0 adds nothing.
    /// Ranges follow mpv's own mapping of -100..=100: brightness and contrast to ffmpeg's
    /// -1..1 and 0..2, saturation to 0..2, gamma to 1/8..8, hue to -180..180 degrees.
    pub fn vf(&self) -> String {
        let [brightness, saturation, contrast, gamma, hue] = self
            .filters
            .map(|v| f64::from(clamp_filter(i64::from(v)) as i32));
        let mut eq: Vec<String> = Vec::new();
        if brightness != 0.0 {
            eq.push(format!("brightness={}", trimmed(brightness / 100.0)));
        }
        if contrast != 0.0 {
            eq.push(format!("contrast={}", trimmed(1.0 + contrast / 100.0)));
        }
        if saturation != 0.0 {
            eq.push(format!("saturation={}", trimmed(1.0 + saturation / 100.0)));
        }
        if gamma != 0.0 {
            eq.push(format!("gamma={}", trimmed(8f64.powf(gamma / 100.0))));
        }
        let mut vf: Vec<String> = Vec::new();
        if self.debanding == Debanding::Cpu {
            vf.push("gradfun=radius=12".to_owned());
        }
        if !eq.is_empty() {
            vf.push(format!("eq={}", eq.join(":")));
        }
        if hue != 0.0 {
            vf.push(format!("hue=h={}", trimmed(hue * 1.8)));
        }
        if self.use_yuv420p {
            vf.push("format=yuv420p".to_owned());
        }
        vf.join(",")
    }
}

/// Builds the ordered startup table of behaviour doc 2.2 (including the subtitle and audio
/// groups, and the two margin options from `PlayerActivity.kt:438-439`).
pub fn startup_settings(p: &PlayerPrefs) -> Vec<MpvSetting> {
    let mut out = Vec::new();
    let mut set = |name: &'static str, value: String| out.push(MpvSetting { name, value });

    // Config first (topic doc section 3 step 1): mpv reads `mpv.conf` and `input.conf` from the
    // config dir (`ensure_mpv_config` creates the files if missing), and only when `config` is on.
    set("config", "yes".to_owned());
    set("config-dir", path_text(&p.data_dir.join("mpv")));
    set("vo", if p.gpu_next { "gpu-next" } else { "gpu" }.to_owned());
    set("pause", "yes".to_owned());
    set("profile", "fast".to_owned());
    let hwdec = match p.remembered_decoder {
        Some(decoder) => resolve_hwdec(decoder, &p.hwdec_available),
        None if p.try_hwdec => resolve_hwdec(Decoder::Auto, &p.hwdec_available),
        None => "no".to_owned(),
    };
    set("hwdec", hwdec);

    // Debanding, the video filters and the yuv420p format share one `vf` list. Kotlin overwrote
    // `vf` in that case (behaviour 2.2 defect); the joined list keeps all of them.
    if p.debanding == Debanding::Gpu {
        set("deband", "yes".to_owned());
    }
    let vf = VideoChain::from(p).vf();
    if !vf.is_empty() {
        set("vf", vf);
    }

    set(
        "msg-level",
        if p.verbose_logging {
            "all=v"
        } else {
            "all=warn"
        }
        .to_owned(),
    );
    set("keep-open", "yes".to_owned());
    set("input-default-bindings", "yes".to_owned());
    set("ytdl", "no".to_owned());
    set("tls-verify", "yes".to_owned());
    set(
        "tls-ca-file",
        path_text(&p.data_dir.join("mpv").join("cacert.pem")),
    );
    set("demuxer-max-bytes", CACHE_BYTES.to_string());
    set("demuxer-max-back-bytes", CACHE_BYTES.to_string());
    set("screenshot-directory", path_text(&p.screenshot_dir));
    set("speed", kotlin_f32(p.player_speed));
    // Workaround for mpv issue 14651 (`AniyomiMPVView.kt:150-152`).
    set("vd-lavc-film-grain", "cpu".to_owned());

    push_subtitles(&p.sub, &mut set);
    push_audio(&p.audio, &mut set);

    set("sub-ass-force-margins", "yes".to_owned());
    set("sub-use-margins", "yes".to_owned());
    let (channels_name, channels_value) = p.audio.channels.setting();
    set(channels_name, channels_value.to_owned());
    out
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn push_subtitles(s: &SubtitlePrefs, set: &mut impl FnMut(&'static str, String)) {
    set("slang", s.lang.clone());
    set("sub-delay", kotlin_f64(f64::from(s.delay_ms) / 1000.0));
    set("sub-speed", kotlin_f32(s.speed));
    set(
        "secondary-sub-delay",
        kotlin_f64(f64::from(s.secondary_delay_ms) / 1000.0),
    );
    set("sub-font", s.font.clone());
    if s.override_ass {
        set("sub-ass-override", "force".to_owned());
    }
    // E27 decision (T-46b-041): a justification picked in the typography card applies to ASS
    // subtitles after a restart, not only with the ASS override.
    if s.override_ass || s.justify != SubtitleJustification::Auto {
        set("sub-ass-justify", "yes".to_owned());
    }
    set("sub-font-size", s.font_size.to_string());
    set("sub-bold", yes_no(s.bold));
    set("sub-italic", yes_no(s.italic));
    set("sub-justify", s.justify.value().to_owned());
    set("sub-color", color_hex(s.text_color_argb));
    set("sub-back-color", color_hex(s.back_color_argb));
    set("sub-border-color", color_hex(s.border_color_argb));
    set("sub-border-size", s.border_size.to_string());
    set("sub-border-style", s.border_style.value().to_owned());
    set("sub-shadow-offset", s.shadow_offset.to_string());
    set("sub-pos", s.pos.to_string());
    set("sub-scale", kotlin_f32(s.scale));
}

fn push_audio(a: &AudioPrefs, set: &mut impl FnMut(&'static str, String)) {
    set("alang", a.lang.clone());
    set("audio-delay", kotlin_f64(f64::from(a.delay_ms) / 1000.0));
    set("audio-pitch-correction", yes_no(a.pitch_correction));
    set("volume-max", (a.volume_boost_cap + 100).to_string());
}

fn yes_no(v: bool) -> String {
    if v { "yes" } else { "no" }.to_owned()
}

/// Looks a setting up by name in an ordered table (the last value wins, as mpv does).
pub fn setting_value<'a>(settings: &'a [MpvSetting], name: &str) -> Option<&'a str> {
    settings
        .iter()
        .rev()
        .find(|s| s.name == name)
        .map(|s| s.value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> PlayerPrefs {
        PlayerPrefs {
            data_dir: PathBuf::from("/data"),
            screenshot_dir: PathBuf::from("/pics"),
            ..PlayerPrefs::default()
        }
    }

    fn pairs(list: &[MpvSetting]) -> Vec<(&'static str, String)> {
        list.iter().map(|s| (s.name, s.value.clone())).collect()
    }

    fn golden_defaults() -> Vec<(&'static str, &'static str)> {
        vec![
            ("config", "yes"),
            ("config-dir", "/data/mpv"),
            ("vo", "gpu"),
            ("pause", "yes"),
            ("profile", "fast"),
            ("hwdec", "auto"),
            ("vf", "format=yuv420p"),
            ("msg-level", "all=warn"),
            ("keep-open", "yes"),
            ("input-default-bindings", "yes"),
            ("ytdl", "no"),
            ("tls-verify", "yes"),
            ("tls-ca-file", "/data/mpv/cacert.pem"),
            ("demuxer-max-bytes", "67108864"),
            ("demuxer-max-back-bytes", "67108864"),
            ("screenshot-directory", "/pics"),
            ("speed", "1.0"),
            ("vd-lavc-film-grain", "cpu"),
            ("slang", ""),
            ("sub-delay", "0.0"),
            ("sub-speed", "1.0"),
            ("secondary-sub-delay", "0.0"),
            ("sub-font", "Sans Serif"),
            ("sub-font-size", "55"),
            ("sub-bold", "no"),
            ("sub-italic", "no"),
            ("sub-justify", "auto"),
            ("sub-color", "#FFFFFFFF"),
            ("sub-back-color", "#00000000"),
            ("sub-border-color", "#FF000000"),
            ("sub-border-size", "3"),
            ("sub-border-style", "outline-and-shadow"),
            ("sub-shadow-offset", "0"),
            ("sub-pos", "100"),
            ("sub-scale", "1.0"),
            ("alang", ""),
            ("audio-delay", "0.0"),
            ("audio-pitch-correction", "yes"),
            ("volume-max", "130"),
            ("sub-ass-force-margins", "yes"),
            ("sub-use-margins", "yes"),
            ("audio-channels", "auto"),
        ]
    }

    #[test]
    fn startup_defaults_match_golden() {
        let got = pairs(&startup_settings(&prefs()));
        // the paths under the data folder are joined by the system (`/data\mpv` on Windows)
        let data = |parts: &[&str]| path_text(&parts.iter().fold(PathBuf::from("/data"), |path, part| path.join(part)));
        let want: Vec<(&str, String)> = golden_defaults()
            .into_iter()
            .map(|(n, v)| match n {
                "config-dir" => (n, data(&["mpv"])),
                "tls-ca-file" => (n, data(&["mpv", "cacert.pem"])),
                _ => (n, v.to_owned()),
            })
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn subtitle_colors_hex() {
        assert_eq!(color_hex(-1), "#FFFFFFFF");
        assert_eq!(color_hex(0), "#00000000");
        assert_eq!(color_hex(-16_777_216), "#FF000000");
    }

    #[test]
    fn font_default_sans_serif() {
        assert_eq!(
            setting_value(&startup_settings(&prefs()), "sub-font"),
            Some("Sans Serif")
        );
    }

    #[test]
    fn cache_limits_are_64_mib() {
        let s = startup_settings(&prefs());
        assert_eq!(setting_value(&s, "demuxer-max-bytes"), Some("67108864"));
        assert_eq!(
            setting_value(&s, "demuxer-max-back-bytes"),
            Some("67108864")
        );
    }
}
