// SPDX-License-Identifier: MIT

//! Platform default Steam roots. Tests must pass an explicit root instead.

use std::path::{Path, PathBuf};

/// Candidate Steam install directories for this OS.
///
/// Windows registry lookups are compiled only on Windows and are **not**
/// covered by the temp-dir fixtures (no live Steam key is read in tests).
#[must_use]
pub fn default_steam_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();

    #[cfg(windows)]
    {
        out.extend(windows_registry_steam_paths());
        out.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    }

    if let Some(home) = home_dir() {
        #[cfg(not(target_os = "macos"))]
        {
            out.push(home.join(".steam").join("steam"));
            out.push(home.join(".local").join("share").join("Steam"));
            out.push(
                home.join(".var")
                    .join("app")
                    .join("com.valvesoftware.Steam")
                    .join(".local")
                    .join("share")
                    .join("Steam"),
            );
        }
        #[cfg(target_os = "macos")]
        {
            out.push(
                home.join("Library")
                    .join("Application Support")
                    .join("Steam"),
            );
        }
    }

    dedup_paths(out)
}

/// Discover using the first existing default root, or a missing-root result.
#[must_use]
pub fn discover_default() -> crate::Discovery {
    for root in default_steam_roots() {
        if root.is_dir() {
            return crate::discover(&root);
        }
    }
    crate::Discovery {
        games: Vec::new(),
        warnings: vec![crate::Warning::new(
            crate::WarningKind::MissingSteamRoot,
            None,
            "no Steam install directory was found",
        )],
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for path in paths {
        let key = std::fs::canonicalize(&path).unwrap_or_else(|_| normalize(&path));
        if seen.iter().any(|s| s == &key) {
            continue;
        }
        seen.push(key);
        out.push(path);
    }
    out
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        out.push(comp);
    }
    out
}

/// HKCU / HKLM `Software\Valve\Steam` `SteamPath` / `InstallPath`.
///
/// `winreg` is used only here, and only on Windows, because that is how Steam
/// records its install location. Tests inject a temp root and never call this.
#[cfg(windows)]
fn windows_registry_steam_paths() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    const SUBKEYS: &[&str] = &[r"Software\Valve\Steam", r"Software\WOW6432Node\Valve\Steam"];
    const VALUES: &[&str] = &["SteamPath", "InstallPath"];

    let hives = [
        RegKey::predef(HKEY_CURRENT_USER),
        RegKey::predef(HKEY_LOCAL_MACHINE),
    ];
    let mut out = Vec::new();
    for hive in &hives {
        for sub in SUBKEYS {
            let Ok(key) = hive.open_subkey_with_flags(sub, KEY_READ) else {
                continue;
            };
            for value_name in VALUES {
                let Ok(value) = key.get_value::<String, _>(value_name) else {
                    continue;
                };
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    out.push(PathBuf::from(trimmed));
                }
            }
        }
    }
    out
}
