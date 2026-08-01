//! Live Patreon supporters fetch for the Start page.
//!
//! The creator access token is injected at BUILD time via the
//! `OCS_PATREON_TOKEN` environment variable (`option_env!`), so it never lives
//! in the source tree or git history — official release builds set it from a CI
//! secret; other builds simply get an empty list. Note that an embedded token
//! can still be extracted from a shipped binary, so it should be a
//! campaign-scoped token with the minimum needed access.

/// Supporters who donated outside Patreon (direct transfer, crypto, one-off
/// gifts, …), maintained by hand here. They are merged with the fetched patrons
/// and ranked together by amount, so the combined Start-page list is ordered by
/// pledge regardless of where the donation came from.
///
/// To add a supporter, add a `("Display name", cents)` line below. The amount is
/// in **cents**: a $25 donation is `2500`, €10 is `1000`.
const MANUAL_SUPPORTERS: &[(&str, i64)] = &[
    ("Stefano", 8750), // $87.50
];

/// Append the hand-maintained [`MANUAL_SUPPORTERS`] to `patrons` and sort the
/// combined list by amount (highest first, then alphabetically). Both the
/// native and web boot paths funnel their fetched list through here, so the
/// manual entries — and the ranking — are identical on every platform, and the
/// list is still sorted (and still shows the manual entries) even when the
/// fetch returns nothing because the app is offline or has no Patreon token.
pub fn merge_manual(mut patrons: Vec<(String, i64)>) -> Vec<(String, i64)> {
    for &(name, cents) in MANUAL_SUPPORTERS {
        let name = name.trim();
        if !name.is_empty() {
            patrons.push((name.to_string(), cents));
        }
    }
    patrons.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    patrons
}

#[cfg(not(target_arch = "wasm32"))]
const UA: &str = concat!("OpenCADStudio/", env!("CARGO_PKG_VERSION"));

/// Fetch the paying patrons from the Patreon API as `(display name, amount in
/// cents)`, highest pledge first. `Err` when no token is configured or the API
/// call fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_patrons() -> Result<Vec<(String, i64)>, String> {
    let token = option_env!("OCS_PATREON_TOKEN")
        .filter(|t| !t.is_empty())
        .ok_or("no Patreon token configured")?;

    let agent = crate::network::agent(std::time::Duration::from_secs(15));

    // The token is creator-scoped, so its first campaign is the one to list.
    let campaigns = get_json(&agent, token, "https://www.patreon.com/api/oauth2/v2/campaigns")?;
    let campaign_id = campaigns["data"][0]["id"]
        .as_str()
        .ok_or("no Patreon campaign found for this token")?
        .to_string();

    // Page through the campaign members, keeping only paying patrons.
    let mut patrons: Vec<(String, i64)> = Vec::new();
    let mut url = format!(
        "https://www.patreon.com/api/oauth2/v2/campaigns/{campaign_id}/members\
         ?fields%5Bmember%5D=full_name,patron_status,currently_entitled_amount_cents\
         &page%5Bcount%5D=200"
    );
    // Bound the loop so a malformed `next` link can never spin forever.
    for _ in 0..50 {
        let page = get_json(&agent, token, &url)?;
        if let Some(arr) = page["data"].as_array() {
            for m in arr {
                let attrs = &m["attributes"];
                // Paying supporters only: an active patron currently entitled to
                // a non-zero amount (excludes free followers, $0 tiers, declined
                // and former patrons).
                if attrs["patron_status"].as_str() != Some("active_patron") {
                    continue;
                }
                let cents = attrs["currently_entitled_amount_cents"].as_i64().unwrap_or(0);
                if cents <= 0 {
                    continue;
                }
                let name = attrs["full_name"].as_str().unwrap_or("").trim();
                if !name.is_empty() {
                    patrons.push((name.to_string(), cents));
                }
            }
        }
        match page["links"]["next"].as_str() {
            Some(next) if !next.is_empty() => url = next.to_string(),
            _ => break,
        }
    }

    // Highest pledge first, then alphabetical.
    patrons.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(patrons)
}

/// Web build: the browser can't call the Patreon API directly (CORS + the
/// token would be exposed in the bundle), so it fetches a pre-generated
/// `supporters.json` published next to the app on the same origin (produced by
/// CI with the token held server-side). Shape: `[{ "name": .., "cents": .. }]`.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_patrons_web() -> Result<Vec<(String, i64)>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or("no window")?;
    let resp_val = JsFuture::from(window.fetch_with_str("supporters.json"))
        .await
        .map_err(|_| "fetch failed")?;
    let resp: web_sys::Response = resp_val.dyn_into().map_err(|_| "not a Response")?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = JsFuture::from(resp.text().map_err(|_| "text() unavailable")?)
        .await
        .map_err(|_| "body read failed")?;
    let body = text.as_string().ok_or("body is not a string")?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let arr = json.as_array().ok_or("supporters.json is not an array")?;
    Ok(arr
        .iter()
        .filter_map(|e| {
            let name = e["name"].as_str()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            Some((name, e["cents"].as_i64().unwrap_or(0)))
        })
        .collect())
}

#[cfg(not(target_arch = "wasm32"))]
fn get_json(
    agent: &ureq::Agent,
    token: &str,
    url: &str,
) -> Result<serde_json::Value, String> {
    let body = agent
        .get(url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("User-Agent", UA)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}
