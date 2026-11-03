//! Desktop notices for saved pictures.
//!
//! Every save shares one notice id, so a new save replaces the notice of the previous one. The `Notifier` trait only
//! takes a title and a body, so the preview and the open and share actions of the Android notice are not sent. The
//! UI can add them through its own notifier later.

use kuro_platform::notifier::Notifier;

use std::path::{Path, PathBuf};

use kuro_platform::paths::Paths;

use crate::error::ImageError;
use crate::messages::Message;
use crate::save::{SaveRequest, save_image};

/// The notice id shared by every picture save (`ID_DOWNLOAD_IMAGE`).
pub const PICTURE_NOTICE_ID: &str = "download-image";

/// Withdraws the previous picture notice. Called at the start of every save.
pub fn clear(notifier: &dyn Notifier) {
    notifier.withdraw(PICTURE_NOTICE_ID);
}

/// Shows "Picture saved". The body is left out, as the Android notice has none.
pub fn saved(notifier: &dyn Notifier) {
    notifier.show(PICTURE_NOTICE_ID, &Message::PictureSaved.text(), None);
}

/// Shows "Error" with the error text, or "Unknown error" when there is none.
pub fn failed(notifier: &dyn Notifier, error: Option<&str>) {
    notifier.show(
        PICTURE_NOTICE_ID,
        &Message::Error.text(),
        Some(error.unwrap_or(&Message::UnknownError.text())),
    );
}

/// Saves the image and sends the matching notice: clear, save, then saved or failed.
pub async fn save_with_notice(
    paths: &dyn Paths,
    pictures_dir: &Path,
    notifier: &dyn Notifier,
    request: SaveRequest,
) -> Result<PathBuf, ImageError> {
    clear(notifier);
    match save_image(paths, pictures_dir, request).await {
        Ok(path) => {
            saved(notifier);
            Ok(path)
        }
        Err(error) => {
            failed(notifier, Some(&error.to_string()));
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::{SaveSource, SaveTarget};
    use crate::testing::{TestPaths, encode, gradient};
    use bytes::Bytes;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<String>>,
    }

    impl Notifier for Recorder {
        fn show(&self, id: &str, title: &str, body: Option<&str>) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("show {id} {title} {}", body.unwrap_or("<none>")));
        }
        fn withdraw(&self, id: &str) {
            self.calls.lock().unwrap().push(format!("withdraw {id}"));
        }
    }

    #[test]
    fn saved_notice_has_title_and_no_body() {
        let rec = Recorder::default();
        saved(&rec);
        assert_eq!(
            rec.calls.lock().unwrap().as_slice(),
            ["show download-image Picture saved <none>"]
        );
    }

    #[test]
    fn failed_notice_uses_the_text_or_unknown_error() {
        let rec = Recorder::default();
        failed(&rec, Some("Error saving picture"));
        failed(&rec, None);
        assert_eq!(
            rec.calls.lock().unwrap().as_slice(),
            [
                "show download-image Error Error saving picture",
                "show download-image Error Unknown error",
            ]
        );
    }

    #[tokio::test]
    async fn a_save_clears_then_replaces_the_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let rec = Recorder::default();
        let png = encode(&gradient(4, 4), image::ImageFormat::Png);
        let request = SaveRequest {
            name: "p".into(),
            source: SaveSource::Page(Bytes::from(png)),
            target: SaveTarget::Pictures { folder: None },
        };
        save_with_notice(&paths, tmp.path(), &rec, request)
            .await
            .unwrap();
        assert_eq!(
            rec.calls.lock().unwrap().as_slice(),
            [
                "withdraw download-image",
                "show download-image Picture saved <none>",
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_save_shows_the_error_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let rec = Recorder::default();
        let request = SaveRequest {
            name: "p".into(),
            source: SaveSource::Page(Bytes::from_static(b"no picture here")),
            target: SaveTarget::Pictures { folder: None },
        };
        assert!(
            save_with_notice(&paths, tmp.path(), &rec, request)
                .await
                .is_err()
        );
        assert_eq!(
            rec.calls.lock().unwrap().as_slice(),
            [
                "withdraw download-image",
                "show download-image Error Not an image",
            ]
        );
    }
}
