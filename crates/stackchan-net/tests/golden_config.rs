//! Host-side integration tests for the RON config schema v1.
//!
//! Layout mirrors `stackchan-sim`'s `tests/` dir convention. Fixtures
//! under `tests/fixtures/` are `include_str!`'d so the binary owns
//! them — a missing file fails the build, not the test.

#![allow(clippy::unwrap_used, clippy::panic, missing_docs)]

use stackchan_net::{Config, ConfigError, parse_ron, render_ron};

const MINIMAL_RON: &str = include_str!("fixtures/minimal.ron");
const FULL_RON: &str = include_str!("fixtures/full.ron");
const EMPTY_SSID_RON: &str = include_str!("fixtures/empty-ssid.ron");
const BAD_COUNTRY_RON: &str = include_str!("fixtures/bad-country.ron");
const BAD_HOSTNAME_RON: &str = include_str!("fixtures/bad-hostname.ron");
const NO_SNTP_SERVERS_RON: &str = include_str!("fixtures/no-sntp-servers.ron");

#[test]
fn parses_minimal_fixture() {
    let cfg = parse_ron(MINIMAL_RON).unwrap();
    assert_eq!(cfg.wifi.ssid, "home");
    assert_eq!(cfg.wifi.psk, "redacted");
    assert_eq!(cfg.wifi.country, "US");
    assert_eq!(cfg.mdns.hostname, "stackchan");
    assert_eq!(cfg.time.tz, "UTC");
    assert_eq!(cfg.time.sntp_servers, vec!["pool.ntp.org".to_string()]);
}

#[test]
fn parses_full_fixture() {
    let cfg = parse_ron(FULL_RON).unwrap();
    assert_eq!(cfg.wifi.ssid, "andy-iot");
    assert_eq!(cfg.wifi.country, "JP");
    assert_eq!(cfg.mdns.hostname, "kai-stackchan-01");
    assert_eq!(cfg.time.tz, "America/Los_Angeles");
    // SNTP server ordering must be preserved end-to-end so the
    // firmware's "try in order" loop hits the user's preferred
    // servers first.
    assert_eq!(
        cfg.time.sntp_servers,
        vec![
            "time.nist.gov".to_string(),
            "time.cloudflare.com".to_string(),
            "pool.ntp.org".to_string(),
        ]
    );
}

#[test]
fn roundtrip_preserves_values() {
    let original = parse_ron(FULL_RON).unwrap();
    let rendered = render_ron(&original).unwrap();
    let reparsed = parse_ron(&rendered).unwrap();
    assert_eq!(original, reparsed);
}

#[test]
fn default_config_does_not_validate_through_parse_ron() {
    // Defaults have an empty SSID — they're the firmware fallback for
    // "no SD / no config", not a thing you'd ever write to disk and
    // re-parse. Confirm: rendering and reparsing the default fails
    // the validator with `EmptySsid`.
    let default = Config::default();
    let rendered = render_ron(&default).unwrap();
    let err = parse_ron(&rendered).unwrap_err();
    assert!(matches!(err, ConfigError::EmptySsid), "got {err:?}");
}

#[test]
fn rejects_empty_ssid() {
    let err = parse_ron(EMPTY_SSID_RON).unwrap_err();
    assert!(matches!(err, ConfigError::EmptySsid), "got {err:?}");
}

#[test]
fn rejects_bad_country() {
    let err = parse_ron(BAD_COUNTRY_RON).unwrap_err();
    match err {
        ConfigError::InvalidCountry(c) => assert_eq!(c, "USA"),
        other => panic!("expected InvalidCountry, got {other:?}"),
    }
}

#[test]
fn rejects_bad_hostname() {
    let err = parse_ron(BAD_HOSTNAME_RON).unwrap_err();
    match err {
        ConfigError::InvalidHostname(h) => assert_eq!(h, "stack_chan"),
        other => panic!("expected InvalidHostname, got {other:?}"),
    }
}

#[test]
fn rejects_empty_sntp_servers() {
    let err = parse_ron(NO_SNTP_SERVERS_RON).unwrap_err();
    assert!(matches!(err, ConfigError::NoSntpServers), "got {err:?}");
}

#[test]
fn rejects_malformed_ron() {
    let err = parse_ron("not valid ron at all").unwrap_err();
    assert!(matches!(err, ConfigError::Parse(_)), "got {err:?}");
}
