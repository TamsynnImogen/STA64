//! MP4 movies (MED-02). Seam placeholder: the demuxer and codecs land with MED-02.

use crate::{MediaError, MediaEvent, MediaSource, ReadSeek, Result, StreamInfo};
use std::time::Duration;

/// An MP4 movie being decoded.
pub struct Mp4Source {
    info: StreamInfo,
}

impl Mp4Source {
    /// Read the container header from `reader`.
    pub fn open(reader: Box<dyn ReadSeek>) -> Result<Self> {
        let _ = reader;
        Err(MediaError::Unsupported("MP4 decoding is not implemented yet".into()))
    }
}

impl MediaSource for Mp4Source {
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
