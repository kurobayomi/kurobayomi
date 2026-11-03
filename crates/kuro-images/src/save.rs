//! Saving images to the pictures folder or the share staging folder.
//!
//! Page bytes are written as they are. A decoded cover is re-encoded as JPEG quality 100.
//! A write goes to `<name>.part` and is renamed over the target, so an existing file of the same name is replaced.
//! The pictures folder comes from the caller, because `Paths` has no pictures getter.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use image::ExtendedColorType;
use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use kuro_platform::paths::Paths;

use crate::cover_file::write_atomic;
use crate::decode::Decoded;
use crate::error::ImageError;
use crate::names::build_valid_filename;
use crate::share::staging_dir;
use crate::sniff::sniff;

/// The app folder under the pictures folder.
pub const PICTURES_FOLDER: &str = "Kurobayomi";

/// Where a saved image goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// `<pictures>/Kurobayomi/<folder>`, or `<pictures>/Kurobayomi` without a folder (folder per title).
    Pictures { folder: Option<String> },
    /// `<cache>/shared_image`. The folder is emptied first, so only the latest share is kept.
    Staging,
}

/// What is written.
#[derive(Clone, Debug)]
pub enum SaveSource {
    /// The original bytes, not re-encoded.
    Page(Bytes),
    /// A decoded image, re-encoded as JPEG quality 100.
    Cover(Arc<Decoded>),
}

#[derive(Clone, Debug)]
pub struct SaveRequest {
    /// The name without an extension. It is sanitised together with the extension.
    pub name: String,
    pub source: SaveSource,
    pub target: SaveTarget,
}

/// Saves the image and returns the absolute path of the file. `pictures_dir` is the user's pictures folder.
pub async fn save_image(
    paths: &dyn Paths,
    pictures_dir: &Path,
    request: SaveRequest,
) -> Result<PathBuf, ImageError> {
    let (bytes, extension) = match request.source {
        SaveSource::Page(bytes) => {
            let format = sniff(&bytes).ok_or(ImageError::NotAnImage)?;
            (bytes, format.extension())
        }
        SaveSource::Cover(decoded) => (Bytes::from(encode_cover_jpeg(&decoded)?), "jpg"),
    };
    let file_name = build_valid_filename(&format!("{}.{extension}", request.name));
    let folder = match request.target {
        SaveTarget::Pictures { folder } => {
            let mut dir = pictures_dir.join(PICTURES_FOLDER);
            if let Some(folder) = folder {
                dir.push(build_valid_filename(&folder));
            }
            dir
        }
        SaveTarget::Staging => {
            let dir = staging_dir(paths);
            match tokio::fs::remove_dir_all(&dir).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(ImageError::SaveFailed),
            }
            dir
        }
    };
    let target = folder.join(file_name);
    write_atomic(&target, &bytes)
        .await
        .map_err(|_| ImageError::SaveFailed)?;
    Ok(target)
}

/// The decoded pixels as a JPEG at quality 100. The alpha channel is dropped.
pub fn encode_cover_jpeg(decoded: &Decoded) -> Result<Vec<u8>, ImageError> {
    let pixels = decoded.width as usize * decoded.height as usize;
    if decoded.rgba.len() != pixels * 4 {
        return Err(ImageError::DecodeFailed);
    }
    let rgb: Vec<u8> = decoded
        .rgba
        .chunks_exact(4)
        .flat_map(|px| [px[0], px[1], px[2]])
        .collect();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 100)
        .write_image(&rgb, decoded.width, decoded.height, ExtendedColorType::Rgb8)
        .map_err(|_| ImageError::SaveFailed)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestPaths, encode, gradient};

    fn png() -> Vec<u8> {
        encode(&gradient(8, 6), image::ImageFormat::Png)
    }

    fn decoded(w: u32, h: u32) -> Decoded {
        Decoded {
            width: w,
            height: h,
            rgba: Bytes::from(vec![200u8; (w * h * 4) as usize]),
            is_sampled: false,
        }
    }

    fn request(name: &str, source: SaveSource, target: SaveTarget) -> SaveRequest {
        SaveRequest {
            name: name.into(),
            source,
            target,
        }
    }

    #[tokio::test]
    async fn page_bytes_are_written_unchanged_with_the_sniffed_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let bytes = png();
        let got = save_image(
            &paths,
            &tmp.path().join("Pictures"),
            request(
                "Page 3",
                SaveSource::Page(Bytes::from(bytes.clone())),
                SaveTarget::Pictures {
                    folder: Some("Manga/One".into()),
                },
            ),
        )
        .await
        .unwrap();
        assert_eq!(
            got,
            tmp.path().join("Pictures/Kurobayomi/Manga_One/Page 3.png")
        );
        assert_eq!(std::fs::read(&got).unwrap(), bytes);
    }

    #[tokio::test]
    async fn pictures_without_a_folder_go_to_the_app_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let got = save_image(
            &paths,
            tmp.path(),
            request(
                "cover",
                SaveSource::Page(Bytes::from(png())),
                SaveTarget::Pictures { folder: None },
            ),
        )
        .await
        .unwrap();
        assert_eq!(got, tmp.path().join("Kurobayomi/cover.png"));
    }

    #[tokio::test]
    async fn an_existing_file_of_the_same_name_is_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let target = tmp.path().join("Kurobayomi/one.png");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"old and much longer than the new picture bytes").unwrap();
        let bytes = png();
        let got = save_image(
            &paths,
            tmp.path(),
            request(
                "one",
                SaveSource::Page(Bytes::from(bytes.clone())),
                SaveTarget::Pictures { folder: None },
            ),
        )
        .await
        .unwrap();
        assert_eq!(got, target);
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert!(!tmp.path().join("Kurobayomi/one.png.part").exists());
    }

    #[tokio::test]
    async fn bytes_that_are_not_an_image_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let err = save_image(
            &paths,
            tmp.path(),
            request(
                "text",
                SaveSource::Page(Bytes::from_static(b"plain text, not a picture")),
                SaveTarget::Pictures { folder: None },
            ),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ImageError::NotAnImage));
        assert_eq!(err.to_string(), "Not an image");
        assert!(!tmp.path().join("Kurobayomi").exists());
    }

    #[tokio::test]
    async fn a_write_failure_is_error_saving_picture() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        // A file where the app folder should be blocks the write.
        std::fs::write(tmp.path().join("Kurobayomi"), b"blocker").unwrap();
        let err = save_image(
            &paths,
            tmp.path(),
            request(
                "x",
                SaveSource::Page(Bytes::from(png())),
                SaveTarget::Pictures { folder: None },
            ),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ImageError::SaveFailed));
        assert_eq!(err.to_string(), "Error saving picture");
    }

    #[tokio::test]
    async fn cover_is_reencoded_as_jpeg_quality_100() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let source = Arc::new(decoded(32, 24));
        let got = save_image(
            &paths,
            tmp.path(),
            request(
                "Jo/Jo",
                SaveSource::Cover(source.clone()),
                SaveTarget::Pictures { folder: None },
            ),
        )
        .await
        .unwrap();
        assert_eq!(got, tmp.path().join("Kurobayomi/Jo_Jo.jpg"));
        let saved = std::fs::read(&got).unwrap();
        assert_eq!(sniff(&saved), Some(crate::sniff::ImageFormat::Jpeg));
        let back = crate::decode::decode(&saved, &Default::default()).unwrap();
        assert_eq!((back.width, back.height), (32, 24));
        // A flat colour survives JPEG within a small tolerance.
        assert!(back.rgba.chunks(4).all(|p| p[0].abs_diff(200) < 6));
    }

    #[test]
    fn quality_100_is_larger_than_a_low_quality_encode() {
        let src = gradient(64, 64);
        let decoded = Decoded {
            width: 64,
            height: 64,
            rgba: Bytes::from(src.as_raw().clone()),
            is_sampled: false,
        };
        let high = encode_cover_jpeg(&decoded).unwrap();
        let mut low = Vec::new();
        let rgb = image::DynamicImage::ImageRgba8(src.clone()).to_rgb8();
        JpegEncoder::new_with_quality(&mut low, 30)
            .write_image(rgb.as_raw(), 64, 64, ExtendedColorType::Rgb8)
            .unwrap();
        assert!(high.len() > low.len());
        assert!(high.len() > 1000);
    }

    #[test]
    fn a_short_pixel_buffer_is_refused() {
        let broken = Decoded {
            width: 4,
            height: 4,
            rgba: Bytes::from(vec![0u8; 10]),
            is_sampled: false,
        };
        assert!(matches!(
            encode_cover_jpeg(&broken),
            Err(ImageError::DecodeFailed)
        ));
    }

    #[tokio::test]
    async fn staging_empties_the_folder_before_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let staging = staging_dir(&paths);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("older.png"), b"old share").unwrap();
        let got = save_image(
            &paths,
            tmp.path(),
            request(
                "latest",
                SaveSource::Page(Bytes::from(png())),
                SaveTarget::Staging,
            ),
        )
        .await
        .unwrap();
        assert_eq!(got, staging.join("latest.png"));
        assert!(!staging.join("older.png").exists());
    }

    #[tokio::test]
    async fn fixtures_that_are_not_pictures_are_refused_by_save() {
        for name in ["zero_bytes.bin", "text_named.jpg"] {
            let tmp = tempfile::tempdir().unwrap();
            let paths = TestPaths::new(tmp.path());
            let bytes = crate::testing::fixture(name);
            let err = save_image(
                &paths,
                tmp.path(),
                request(
                    "x",
                    SaveSource::Page(Bytes::from(bytes)),
                    SaveTarget::Pictures { folder: None },
                ),
            )
            .await
            .unwrap_err();
            assert!(matches!(err, ImageError::NotAnImage), "{name}: {err}");
        }
    }
}
