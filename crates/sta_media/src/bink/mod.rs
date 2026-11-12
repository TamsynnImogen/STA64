//! Bink 1 movies (MED-01). Seam placeholder: the decoder lands with MED-01.

use crate::{MediaError, MediaEvent, MediaSource, ReadSeek, Result, StreamInfo};
use std::time::Duration;

/// A Bink movie being decoded.
pub struct BinkSource {
    info: StreamInfo,
}

impl BinkSource {
    /// Read the container header from `reader`.
    pub fn open(reader: Box<dyn ReadSeek>) -> Result<Self> {
        let _ = reader;
        Err(MediaError::Unsupported("Bink decoding is not implemented yet".into()))
    }
}

impl MediaSource for BinkSource {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn next_event(&mut self) -> Result<Option<MediaEvent>> {
        Ok(None)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        Ok(to)
    }
}
