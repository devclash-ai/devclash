use super::*;
use serde_json::json;
#[test]
fn direct_codex_windows_use_reported_duration_and_preserve_zero() {
    let now = Utc::now();
    let weekly=parse("codex",&json!({"rate_limit":{"primary_window":{"used_percent":42,"limit_window_seconds":604800,"reset_after_seconds":3600}}}),now).unwrap();
    assert_eq!(weekly.label, "Weekly");
    assert_eq!(weekly.used, 42.);
    assert!(weekly.reset_label.unwrap().contains("1h"));
    let both=parse("codex",&json!({"rate_limit":{"primary_window":{"used_percent":90,"limit_window_seconds":604800},"secondary_window":{"used_percent":0,"limit_window_seconds":18000}}}),now).unwrap();
    assert_eq!(both.label, "Session");
    assert_eq!(both.used, 0.);
    assert!(parse("codex", &json!({"rate_limit":{"primary_window":{}}}), now).is_none());
}
#[test]
fn provider_adapters_use_supported_percentages_without_inventing_budget() {
    let now = Utc::now();
    let reset = (now + chrono::Duration::hours(2)).to_rfc3339();
    let claude = parse(
        "claude",
        &json!({"five_hour":{"utilization":38,"resets_at":reset}}),
        now,
    )
    .unwrap();
    assert_eq!(claude.used, 38.);
    let cursor = parse(
        "cursor",
        &json!({"billingCycleEnd":reset,"individualUsage":{"plan":{"totalPercentUsed":0}}}),
        now,
    )
    .unwrap();
    assert_eq!(cursor.used, 0.);
    assert_eq!(cursor.label, "Plan");
    assert!(parse(
        "cursor",
        &json!({"individualUsage":{"plan":{"used":10,"limit":100}}}),
        now
    )
    .is_none());
    let go = parse(
        "opencode",
        &json!({"usage":{"weekly":{"percent":40,"resetsAt":reset}}}),
        now,
    )
    .unwrap();
    assert_eq!(go.label, "Go Weekly");
    assert!(parse(
        "opencode",
        &json!({"usage":{"rolling":{"percent":-1}}}),
        now
    )
    .is_none());
}
#[test]
fn revoked_quota_login_has_a_friendly_reason_without_erasing_local_tokens() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/usage", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let _ = stream.read(&mut request);
        stream
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
            .unwrap();
    });
    assert!(fetch(&url, HeaderMap::new())
        .unwrap_err()
        .contains("Sign in"));
    server.join().unwrap();
}
