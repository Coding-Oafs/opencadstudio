// build.rs — Windows executable icon embedding.
//
// The core ribbon module registry used to be generated here; it is now a
// hand-written static list in src/modules/registry.rs.

#[cfg(windows)]
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Ok(head) = std::fs::read_to_string(".git/HEAD") {
        if let Some(reference) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=.git/{reference}");
        }
    }
    let revision = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = std::process::Command::new("git")
        .args(["diff-index", "--quiet", "HEAD", "--"])
        .status()
        .ok()
        .is_some_and(|status| !status.success());
    let revision = if dirty {
        format!("{revision}-dirty")
    } else {
        revision
    };
    println!("cargo:rustc-env=OCS_GIT_REV={revision}");
    println!(
        "cargo:rustc-env=OCS_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string())
    );
    let mut features: Vec<String> = std::env::vars()
        .filter_map(|(name, value)| {
            (value == "1")
                .then(|| name.strip_prefix("CARGO_FEATURE_").map(str::to_owned))
                .flatten()
        })
        .collect();
    features.sort();
    println!(
        "cargo:rustc-env=OCS_BUILD_FEATURES={}",
        if features.is_empty() {
            "none".to_string()
        } else {
            features.join(",")
        }
    );

    // The Patreon token is baked in at compile time via `option_env!` in
    // src/patreon.rs. `option_env!` is not tracked by Cargo, so without this a
    // token change wouldn't trigger a rebuild — declare the dependency so an
    // updated OCS_PATREON_TOKEN re-bakes the binary. (#229-adjacent)
    println!("cargo:rerun-if-env-changed=OCS_PATREON_TOKEN");

    // Windows: embed AppIcon.ico into the .exe so the executable carries its
    // own icon (Explorer, taskbar, Start-menu tile, file associations). The
    // .ico is produced from assets/logo.svg by the release workflow before the
    // build; when it is absent (local/dev builds) this is skipped. See #107.
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=packaging/windows/AppIcon.ico");
        if Path::new("packaging/windows/AppIcon.ico").exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("packaging/windows/AppIcon.ico");
            if let Err(e) = res.compile() {
                println!("cargo:warning=failed to embed Windows icon: {e}");
            }
        }
    }
}
