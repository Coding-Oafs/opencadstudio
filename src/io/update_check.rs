// One-shot update check.
//
// `check_for_update()` runs on a background thread (joined inside an
// async wrapper, mirroring how the rest of `crate::io` wraps blocking
// work for iced's `Task::perform`). It hits the GitHub releases API and
// returns `Some(UpdateInfo)` when a newer release is available, or
// `None` when up to date / on network failure / on parse error.

#[cfg(not(target_arch = "wasm32"))]
const RELEASES_API: &str =
    "https://api.github.com/repos/HakanSeven12/OpenCADStudio/releases/latest";
pub const RELEASES_PAGE: &str =
    "https://github.com/HakanSeven12/OpenCADStudio/releases/latest";

/// Minimum age before a freshly-published release is offered to the user.
/// GitHub Actions takes ~15 min to build and attach the platform binaries
/// after a tag is pushed, so suppressing notifications for the first hour
/// prevents users from clicking through to a release page whose
/// asset list is still empty.
#[cfg(not(target_arch = "wasm32"))]
const MIN_RELEASE_AGE_SECS: u64 = 60 * 60;

/// What `check_for_update` reports when a newer release exists.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// `tag_name` with the leading `v` stripped (e.g. `0.3.7`).
    pub version: String,
    /// Release notes / markdown body from the GitHub release. May be empty
    /// when the release was published without notes.
    pub body: String,
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn check_for_update() -> Option<UpdateInfo> {
    std::thread::spawn(fetch_latest_if_outdated)
        .join()
        .ok()
        .flatten()
}

/// On wasm there is no background thread or blocking HTTP client; the web build
/// skips the self-update check (the page is always served fresh).
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
pub async fn check_for_update() -> Option<UpdateInfo> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch_latest_if_outdated() -> Option<UpdateInfo> {
    let agent = crate::network::agent(std::time::Duration::from_secs(5));
    let body = agent
        .get(RELEASES_API)
        .header("User-Agent", concat!("OpenCADStudio/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let latest = extract_string_field(&body, "tag_name")?
        .trim_start_matches('v')
        .to_string();
    if latest == env!("CARGO_PKG_VERSION") {
        return None;
    }
    // Suppress the notification until the release is old enough for the
    // Actions build to have published binaries.
    if let Some(published) = extract_string_field(&body, "published_at")
        .as_deref()
        .and_then(parse_iso8601_utc)
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        if now.saturating_sub(published) < MIN_RELEASE_AGE_SECS {
            return None;
        }
    }
    // Release notes are optional; treat missing as empty.
    let notes = extract_string_field(&body, "body").unwrap_or_default();
    Some(UpdateInfo { version: latest, body: notes })
}

/// Parse a GitHub timestamp like `2026-05-29T12:34:56Z` into UNIX seconds.
/// Only handles the fixed `YYYY-MM-DDTHH:MM:SSZ` format the GitHub API
/// emits; returns `None` for anything else.
#[cfg(not(target_arch = "wasm32"))]
fn parse_iso8601_utc(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T'
        || b[13] != b':' || b[16] != b':' || b[19] != b'Z'
    {
        return None;
    }
    let year: i32 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let hour: u32 = s.get(11..13)?.parse().ok()?;
    let minute: u32 = s.get(14..16)?.parse().ok()?;
    let second: u32 = s.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour >= 24
        || minute >= 60
        || second >= 60
    {
        return None;
    }
    // Howard Hinnant's days_from_civil — converts a proleptic-Gregorian
    // (Y, M, D) to a count of days since 1970-01-01.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u32;
    let m = month as i32;
    let d = day as i32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u32;
    let days_since_epoch = era as i64 * 146097 + doe as i64 - 719468;
    if days_since_epoch < 0 {
        return None;
    }
    Some(
        days_since_epoch as u64 * 86_400
            + hour as u64 * 3_600
            + minute as u64 * 60
            + second as u64,
    )
}

/// Minimal extractor for a top-level string field in the releases JSON.
/// Avoids pulling in `serde_json` for two fields. Handles standard JSON
/// string escapes (`\"`, `\\`, `\n`, `\r`, `\t`, `\/`) which are all the
/// GitHub release body uses in practice.
#[cfg(not(target_arch = "wasm32"))]
fn extract_string_field(body: &str, field: &str) -> Option<String> {
    let key = format!("\"{}\":\"", field);
    let start = body.find(&key)? + key.len();
    // Walk to the closing unescaped `"`, JSON-unescaping as we go.
    let mut out = String::new();
    let bytes = body.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            return Some(out);
        }
        if b == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    // \uXXXX — decode 4 hex digits. Surrogate pairs (BMP only
                    // for now) are uncommon in release notes; skip on parse
                    // failure.
                    if i + 5 < bytes.len() {
                        let hex = std::str::from_utf8(&bytes[i + 2..i + 6]).ok()?;
                        if let Ok(code) = u32::from_str_radix(hex, 16) {
                            if let Some(c) = char::from_u32(code) {
                                out.push(c);
                            }
                        }
                        i += 6;
                        continue;
                    }
                    return None;
                }
                other => {
                    // Unknown escape — keep the literal pair, GitHub doesn't emit these.
                    out.push('\\');
                    out.push(other as char);
                }
            }
            i += 2;
            continue;
        }
        // UTF-8 multi-byte chars: push the whole code-point so we don't
        // bisect a sequence.
        let ch = body[i..].chars().next()?;
        out.push(ch);
        i += ch.len_utf8();
    }
    None
}
