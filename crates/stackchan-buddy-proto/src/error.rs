//! Typed errors for the buddy protocol.

use alloc::string::String;

use thiserror::Error;

/// Errors surfaced by [`crate::parse_inbound`] and the framing
/// helpers in [`crate::frame`]. Outbound builders are infallible —
/// the type system carries the schema.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtoError {
    /// The byte slice was not valid UTF-8.
    #[error("invalid utf-8")]
    InvalidUtf8,

    /// The input was not a well-formed JSON object literal (missing
    /// brace, unbalanced quotes, dangling escape, etc.).
    #[error("malformed json: {0}")]
    MalformedJson(&'static str),

    /// The structure parsed, but a required field for the inferred
    /// message kind was missing.
    #[error("missing required field `{0}`")]
    MissingField(&'static str),

    /// A field's value had the wrong shape (string where an integer
    /// was expected, etc.).
    #[error("bad value for `{field}`: {reason}")]
    BadValue {
        /// JSON key whose value was rejected.
        field: &'static str,
        /// Human-readable reason.
        reason: &'static str,
    },

    /// The message could not be classified as any known kind. The
    /// payload (a short slice of the input) is included for debug.
    #[error("unknown message kind: {0}")]
    UnknownKind(String),

    /// A folder-push chunk's base64 payload could not be decoded.
    #[error("invalid base64 in chunk payload")]
    InvalidBase64,
}
