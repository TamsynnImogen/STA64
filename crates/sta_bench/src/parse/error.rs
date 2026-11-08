/// A parse failure with the 1-based line it happened on.
///
/// For a command given on its own (a `Scene "..."` step, `Scene.cmd`), the
/// line is that of the text handed in; callers that know better re-base it
/// with [`ParseError::at_line`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl ParseError {
    pub fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }

    /// The same error attributed to another line, for text that was parsed
    /// as a fragment of a larger file.
    #[must_use]
    pub fn at_line(mut self, line: usize) -> Self {
        self.line = line;
        self
    }
}
