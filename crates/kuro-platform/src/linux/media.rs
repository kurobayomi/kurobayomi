//! MPRIS media controls through `souvlaki`. Media keys and the desktop media widget read the same
//! `org.mpris.MediaPlayer2.<name>` service.

use std::sync::Mutex;
use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};

use crate::media::{CommandHandler, MediaCommand, MediaControls, MediaError, NowPlaying, PlaybackState};

/// The step for a seek event that carries no amount (`MediaControlEvent::Seek`).
pub const DEFAULT_SEEK: Duration = Duration::from_secs(10);

/// MPRIS controls for this process.
pub struct Mpris {
    controls: Mutex<souvlaki::MediaControls>,
}

impl Mpris {
    /// `name` is the MPRIS suffix, so the service is `org.mpris.MediaPlayer2.<name>`. Fails when no session bus is
    /// reachable: `souvlaki` would panic on its own thread without one, which the crash reporter would record.
    pub fn connect(display_name: &str, name: &str) -> Result<Self, MediaError> {
        zbus::blocking::Connection::session().map(drop).map_err(|err| MediaError(err.to_string()))?;
        let config = PlatformConfig { display_name, dbus_name: name, hwnd: None };
        let controls = souvlaki::MediaControls::new(config).map_err(|err| MediaError(err.to_string()))?;
        Ok(Self { controls: Mutex::new(controls) })
    }

    fn with<T>(&self, f: impl FnOnce(&mut souvlaki::MediaControls) -> Result<T, souvlaki::Error>) -> Result<T, MediaError> {
        let mut controls = self.controls.lock().map_err(|_| MediaError("controls lock poisoned".into()))?;
        f(&mut controls).map_err(|err| MediaError(err.to_string()))
    }
}

impl MediaControls for Mpris {
    fn on_command(&self, handler: CommandHandler) -> Result<(), MediaError> {
        self.with(|controls| {
            controls.attach(move |event| {
                if let Some(command) = command_of(event) {
                    handler(command);
                }
            })
        })
    }

    fn set_now_playing(&self, now: &NowPlaying) -> Result<(), MediaError> {
        self.with(|controls| {
            controls.set_metadata(MediaMetadata {
                title: Some(&now.title),
                cover_url: now.artwork.as_deref(),
                duration: now.duration,
                ..Default::default()
            })
        })
    }

    fn set_playback(&self, state: PlaybackState, position: Option<Duration>) -> Result<(), MediaError> {
        let progress = position.map(MediaPosition);
        let playback = match state {
            PlaybackState::Playing => MediaPlayback::Playing { progress },
            PlaybackState::Paused => MediaPlayback::Paused { progress },
            PlaybackState::Stopped => MediaPlayback::Stopped,
        };
        self.with(|controls| controls.set_playback(playback))
    }
}

/// The transport command an MPRIS event means. Events the player has no use for (volume, open URI, raise, quit) map to
/// `None`.
pub fn command_of(event: MediaControlEvent) -> Option<MediaCommand> {
    Some(match event {
        MediaControlEvent::Play => MediaCommand::Play,
        MediaControlEvent::Pause => MediaCommand::Pause,
        MediaControlEvent::Toggle => MediaCommand::Toggle,
        MediaControlEvent::Next => MediaCommand::Next,
        MediaControlEvent::Previous => MediaCommand::Previous,
        MediaControlEvent::Stop => MediaCommand::Stop,
        MediaControlEvent::Seek(SeekDirection::Forward) => MediaCommand::SeekForward(DEFAULT_SEEK),
        MediaControlEvent::Seek(SeekDirection::Backward) => MediaCommand::SeekBackward(DEFAULT_SEEK),
        MediaControlEvent::SeekBy(SeekDirection::Forward, by) => MediaCommand::SeekForward(by),
        MediaControlEvent::SeekBy(SeekDirection::Backward, by) => MediaCommand::SeekBackward(by),
        MediaControlEvent::SetPosition(MediaPosition(at)) => MediaCommand::SeekTo(at),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_events_map_to_commands() {
        assert_eq!(command_of(MediaControlEvent::Play), Some(MediaCommand::Play));
        assert_eq!(command_of(MediaControlEvent::Pause), Some(MediaCommand::Pause));
        assert_eq!(command_of(MediaControlEvent::Toggle), Some(MediaCommand::Toggle));
        assert_eq!(command_of(MediaControlEvent::Next), Some(MediaCommand::Next));
        assert_eq!(command_of(MediaControlEvent::Previous), Some(MediaCommand::Previous));
        assert_eq!(command_of(MediaControlEvent::Stop), Some(MediaCommand::Stop));
    }

    #[test]
    fn seeks_map_to_steps_and_absolute_positions() {
        let five = Duration::from_secs(5);
        assert_eq!(command_of(MediaControlEvent::Seek(SeekDirection::Forward)), Some(MediaCommand::SeekForward(DEFAULT_SEEK)));
        assert_eq!(command_of(MediaControlEvent::SeekBy(SeekDirection::Backward, five)), Some(MediaCommand::SeekBackward(five)));
        assert_eq!(command_of(MediaControlEvent::SetPosition(MediaPosition(five))), Some(MediaCommand::SeekTo(five)));
    }

    #[test]
    fn events_the_player_does_not_use_are_dropped() {
        assert_eq!(command_of(MediaControlEvent::SetVolume(0.5)), None);
        assert_eq!(command_of(MediaControlEvent::Raise), None);
        assert_eq!(command_of(MediaControlEvent::Quit), None);
        assert_eq!(command_of(MediaControlEvent::OpenUri("x".into())), None);
    }
}
