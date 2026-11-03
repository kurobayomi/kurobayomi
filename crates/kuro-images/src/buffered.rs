//! In-memory byte source: page data the reader already holds (T-34a-024).

use bytes::Bytes;

use crate::decode::{DecodeOptions, Decoded, decode};
use crate::error::ImageError;
use crate::fetch::{DataSource, Fetched, Payload};

/// A buffer that is read exactly once. Kotlin wraps a live `BufferedSource`, which a second read finds empty
/// (edge E-IMG-12); here the second use is an error instead of a silent empty image. The data source is
/// `Memory` and there is no MIME type, so the decoder sniffs the bytes.
pub struct BufferedSource {
    bytes: Option<Bytes>,
}

impl BufferedSource {
    pub fn new(bytes: Bytes) -> Self {
        Self { bytes: Some(bytes) }
    }

    /// Hands the buffer over. A second call is `DecodeFailed`.
    pub fn take(&mut self) -> Result<Fetched, ImageError> {
        let bytes = self.bytes.take().ok_or(ImageError::DecodeFailed)?;
        Ok(Fetched {
            source: DataSource::Memory,
            payload: Payload::Bytes(bytes),
        })
    }

    /// Decodes the buffer without touching the disk. This is blocking work: run it on a blocking thread.
    pub fn decode(&mut self, opts: &DecodeOptions) -> Result<Decoded, ImageError> {
        match self.take()?.payload {
            Payload::Bytes(bytes) => decode(&bytes, opts),
            Payload::File(_) => Err(ImageError::InvalidImage),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{encode, gradient};

    fn png() -> Bytes {
        Bytes::from(encode(&gradient(20, 10), image::ImageFormat::Png))
    }

    #[test]
    fn take_reports_memory_and_a_byte_payload() {
        let bytes = png();
        let got = BufferedSource::new(bytes.clone()).take().unwrap();
        assert_eq!(got.source, DataSource::Memory);
        assert_eq!(got.payload, Payload::Bytes(bytes));
    }

    #[test]
    fn decodes_from_memory() {
        let got = BufferedSource::new(png())
            .decode(&DecodeOptions::default())
            .unwrap();
        assert_eq!((got.width, got.height), (20, 10));
    }

    #[test]
    fn the_buffer_is_consumed_exactly_once() {
        let mut source = BufferedSource::new(png());
        assert!(source.decode(&DecodeOptions::default()).is_ok());
        assert!(matches!(
            source.decode(&DecodeOptions::default()),
            Err(ImageError::DecodeFailed)
        ));
        assert!(matches!(source.take(), Err(ImageError::DecodeFailed)));
    }
}
