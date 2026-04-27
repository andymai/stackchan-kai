//! Byte-level helpers for the firmware's HTTP request parser.
//!
//! These live here (not in `stackchan-firmware`) so they get host-side
//! `cargo test` coverage. The firmware crate compiles only for
//! `xtensa-esp32s3-none-elf` and its `cfg(test)` modules are not
//! exercised by `just check` or by CI — keeping the parsing edges in
//! a host-testable crate is the difference between "pinned" and
//! "documented but never run."
//!
//! Functions here are deliberately small and protocol-specific
//! (`parse_content_length`, `parse_bearer_token`, `ct_eq`) plus a few
//! generic byte helpers the firmware currently uses
//! (`find_subsequence`, `split_once`, `trim_ascii`). The firmware's
//! `net::http` module owns the higher-level state machine and maps
//! [`ParseError`] into its own `HttpError` surface.

/// Generic parse failure for the helpers that can fail. Kept unit-
/// shaped because the firmware only ever needs to know whether the
/// input was malformed; the route handler maps to `400 Bad Request`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

/// Parse the `Content-Length` header value out of a header block.
///
/// Returns `Ok(0)` when the header is absent — correct for `GET`
/// and 0-body `POST` requests. Returns [`ParseError`] when the
/// header is present but not a valid non-negative integer.
///
/// Header name compare is case-insensitive (RFC 9110 §5.1). The
/// first match wins on duplicate headers; the firmware's request
/// surface is small enough that strict de-duplication isn't worth
/// the bytes.
///
/// # Errors
///
/// [`ParseError`] when a `Content-Length` header is present but
/// can't be parsed as `usize`.
pub fn parse_content_length(headers: &[u8]) -> Result<usize, ParseError> {
    for line in headers.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some((name, value)) = split_once(line, b':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case(b"content-length") {
            continue;
        }
        let value = trim_ascii(value);
        // RFC 9110 §8.6: `Content-Length = 1*DIGIT`. `usize::from_str`
        // accepts a leading `+`, which would otherwise leak through;
        // enforce digits-only here so the wire shape is spec-tight.
        if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
            return Err(ParseError);
        }
        let s = core::str::from_utf8(value).map_err(|_| ParseError)?;
        return s.parse::<usize>().map_err(|_| ParseError);
    }
    Ok(0)
}

/// Parse the `Authorization: Bearer <token>` header value out of a
/// header block.
///
/// Returns `Some(token)` if a Bearer credential is present, `None`
/// otherwise. Header name compare is case-insensitive (RFC 9110 §5.1).
/// Scheme compare is also case-insensitive — RFC 6750 says the
/// `Authorization` field uses the standard `auth-scheme` token
/// production, which is case-insensitive.
///
/// Whitespace between scheme and token is collapsed; leading and
/// trailing whitespace on the token are stripped. The token itself
/// is not further validated; the caller compares against the
/// configured value with [`ct_eq`].
#[must_use]
pub fn parse_bearer_token(headers: &[u8]) -> Option<&str> {
    for line in headers.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some((name, value)) = split_once(line, b':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case(b"authorization") {
            continue;
        }
        let value = trim_ascii(value);
        let scheme_end = value.iter().position(|&b| b == b' ')?;
        let (scheme, rest) = value.split_at(scheme_end);
        if !scheme.eq_ignore_ascii_case(b"bearer") {
            return None;
        }
        let token = trim_ascii(rest);
        return core::str::from_utf8(token).ok();
    }
    None
}

/// Constant-time byte comparison.
///
/// Folds every byte difference into a single accumulator before
/// testing equality so timing leaks from an early-exit-on-mismatch
/// loop can't reveal a prefix of the expected token. Length mismatch
/// returns immediately — leaking the token *length* is acceptable;
/// the operator chose it.
#[must_use]
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Index of the first occurrence of `needle` in `haystack`, or `None`.
///
/// `needle` must be non-empty — `slice::windows(0)` panics. The
/// firmware's only caller passes the four-byte CRLF CRLF sentinel,
/// so the constraint is upheld at every call site.
#[must_use]
pub fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Split `slice` at the first occurrence of `delim`. Returns `None`
/// if the delimiter is absent.
#[must_use]
pub fn split_once(slice: &[u8], delim: u8) -> Option<(&[u8], &[u8])> {
    let idx = slice.iter().position(|&b| b == delim)?;
    Some((&slice[..idx], &slice[idx + 1..]))
}

/// Strip ASCII whitespace from both ends of `slice`.
#[must_use]
pub fn trim_ascii(slice: &[u8]) -> &[u8] {
    let start = slice
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(slice.len());
    let end = slice
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    &slice[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_length_missing_defaults_to_zero() {
        let headers = b"Host: stackchan.local\r\nUser-Agent: curl/8\r\n";
        assert_eq!(parse_content_length(headers), Ok(0));
    }

    #[test]
    fn content_length_is_case_insensitive() {
        for raw in [
            "Content-Length: 42\r\n",
            "content-length: 42\r\n",
            "CONTENT-LENGTH: 42\r\n",
            "cOnTeNt-LeNgTh: 42\r\n",
        ] {
            assert_eq!(parse_content_length(raw.as_bytes()), Ok(42), "{raw}");
        }
    }

    #[test]
    fn content_length_trims_value_whitespace() {
        let headers = b"Content-Length:    99\r\n";
        assert_eq!(parse_content_length(headers), Ok(99));
    }

    #[test]
    fn content_length_rejects_non_numeric() {
        let headers = b"Content-Length: forty-two\r\n";
        assert_eq!(parse_content_length(headers), Err(ParseError));
    }

    #[test]
    fn content_length_rejects_signed_value() {
        // `usize::from_str` doesn't accept a leading `+` or `-`; pin
        // that the parser surfaces it as Malformed rather than 0.
        for raw in ["Content-Length: +5\r\n", "Content-Length: -5\r\n"] {
            assert_eq!(
                parse_content_length(raw.as_bytes()),
                Err(ParseError),
                "{raw}"
            );
        }
    }

    #[test]
    fn content_length_rejects_overflow() {
        // 2^64-ish — never fits in usize on 32- or 64-bit targets.
        let headers = b"Content-Length: 99999999999999999999\r\n";
        assert_eq!(parse_content_length(headers), Err(ParseError));
    }

    #[test]
    fn content_length_first_match_wins_on_duplicates() {
        // RFC 7230 forbids multiple Content-Length headers with
        // different values; this server takes the first match. Pin
        // the behaviour so a future change is conscious.
        let headers = b"Content-Length: 7\r\nContent-Length: 99\r\n";
        assert_eq!(parse_content_length(headers), Ok(7));
    }

    #[test]
    fn bearer_token_extracts_value() {
        let headers = b"Host: stackchan.local\r\nAuthorization: Bearer abc123\r\n";
        assert_eq!(parse_bearer_token(headers), Some("abc123"));
    }

    #[test]
    fn bearer_scheme_and_header_are_case_insensitive() {
        for raw in [
            "authorization: Bearer xyz\r\n",
            "AUTHORIZATION: BEARER xyz\r\n",
            "Authorization: bearer xyz\r\n",
        ] {
            assert_eq!(parse_bearer_token(raw.as_bytes()), Some("xyz"), "{raw}");
        }
    }

    #[test]
    fn bearer_token_absent_returns_none() {
        let headers = b"Host: stackchan.local\r\nUser-Agent: curl/8\r\n";
        assert_eq!(parse_bearer_token(headers), None);
    }

    #[test]
    fn bearer_token_rejects_other_schemes() {
        for raw in [
            "Authorization: Basic dXNlcjpwYXNz\r\n",
            "Authorization: Digest realm=test\r\n",
        ] {
            assert_eq!(parse_bearer_token(raw.as_bytes()), None, "{raw}");
        }
    }

    #[test]
    fn bearer_token_handles_extra_whitespace() {
        let headers = b"Authorization:    Bearer    spaced-token   \r\n";
        assert_eq!(parse_bearer_token(headers), Some("spaced-token"));
    }

    #[test]
    fn ct_eq_reports_equality_and_diff() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(!ct_eq(b"", b"a"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn find_subsequence_locates_or_misses() {
        assert_eq!(find_subsequence(b"abcdef", b"cd"), Some(2));
        assert_eq!(find_subsequence(b"abcdef", b"xy"), None);
        assert_eq!(find_subsequence(b"abc", b"abcd"), None);
    }

    #[test]
    fn split_once_partitions_on_first_delim() {
        assert_eq!(
            split_once(b"key: value", b':'),
            Some((b"key".as_slice(), b" value".as_slice()))
        );
        assert_eq!(
            split_once(b"a:b:c", b':'),
            Some((b"a".as_slice(), b"b:c".as_slice()))
        );
        assert_eq!(
            split_once(b":empty", b':'),
            Some((b"".as_slice(), b"empty".as_slice()))
        );
        assert_eq!(split_once(b"none", b':'), None);
    }

    #[test]
    fn trim_ascii_strips_both_sides() {
        assert_eq!(trim_ascii(b"  hello  "), b"hello");
        assert_eq!(trim_ascii(b"\t\rkey\n "), b"key");
        assert_eq!(trim_ascii(b"   "), b"");
        assert_eq!(trim_ascii(b"clean"), b"clean");
    }
}
