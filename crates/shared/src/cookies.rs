use anyhow::{Context, Result};
use cookie_store::CookieStore;
use rusqlite::Connection;
use std::path::PathBuf;
use url::Url;

pub fn load_browser_cookies() -> Result<CookieStore> {
    let mut cookie_store = CookieStore::default();

    if let Some(firefox_path) = find_firefox_cookies() {
        match load_firefox_cookies_from_db(&firefox_path, &mut cookie_store) {
            Ok(count) if count > 0 => {
                eprintln!("✓ Loaded {} cookies from Firefox", count);
            }
            Ok(_) => {
                eprintln!("  Note: Found Firefox cookies but loaded 0");
            }
            Err(e) => {
                eprintln!("  Warning: Could not load Firefox cookies: {}", e);
            }
        }
    } else {
        eprintln!("  Note: No Firefox cookies found (paywalled sites may not work)");
    }

    Ok(cookie_store)
}

fn find_firefox_cookies() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let firefox_dir = home.join(".mozilla/firefox");

    if !firefox_dir.exists() {
        return None;
    }

    // Look for profiles.ini to find the default profile
    let profiles_ini = firefox_dir.join("profiles.ini");
    if profiles_ini.exists() {
        if let Ok(content) = std::fs::read_to_string(&profiles_ini) {
            let mut current_path: Option<String> = None;
            let mut is_default = false;

            for line in content.lines() {
                if line.starts_with("Path=") {
                    current_path = Some(line.trim_start_matches("Path=").to_string());
                }
                if line == "Default=1" {
                    is_default = true;
                }
                if line.starts_with('[') && line != "[General]" {
                    if is_default {
                        if let Some(path) = current_path {
                            let profile_dir = firefox_dir.join(&path);
                            let cookies_path = profile_dir.join("cookies.sqlite");
                            if cookies_path.exists() {
                                return Some(cookies_path);
                            }
                        }
                    }
                    current_path = None;
                    is_default = false;
                }
            }

            // Check last section
            if is_default {
                if let Some(path) = current_path {
                    let profile_dir = firefox_dir.join(&path);
                    let cookies_path = profile_dir.join("cookies.sqlite");
                    if cookies_path.exists() {
                        return Some(cookies_path);
                    }
                }
            }
        }
    }

    // Fallback: find any profile with cookies.sqlite
    if let Ok(entries) = std::fs::read_dir(&firefox_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let cookies_path = path.join("cookies.sqlite");
                if cookies_path.exists() {
                    return Some(cookies_path);
                }
            }
        }
    }

    None
}

fn load_firefox_cookies_from_db(
    db_path: &PathBuf,
    cookie_store: &mut CookieStore,
) -> Result<usize> {
    // Firefox locks the database, so we need to copy it first
    let temp_path = std::env::temp_dir().join("collect-stories-firefox-cookies.db");

    // Copy the database to avoid locking issues
    std::fs::copy(db_path, &temp_path).context("Failed to copy Firefox cookies database")?;

    let conn = Connection::open(&temp_path).context("Failed to open Firefox cookies database")?;

    // Current time in Unix timestamp (seconds)
    let now = chrono::Utc::now().timestamp();

    let mut stmt = conn.prepare(
        "SELECT host, path, isSecure, expiry, name, value, isHttpOnly
         FROM moz_cookies
         WHERE expiry > ? AND name != '' AND value != ''",
    )?;

    let mut count = 0;
    let rows = stmt.query_map([now], |row| {
        Ok((
            row.get::<_, String>(0)?, // host
            row.get::<_, String>(1)?, // path
            row.get::<_, i64>(2)?,    // isSecure
            row.get::<_, i64>(3)?,    // expiry
            row.get::<_, String>(4)?, // name
            row.get::<_, String>(5)?, // value
            row.get::<_, i64>(6)?,    // isHttpOnly
        ))
    })?;

    for (host, path, is_secure, _expires, name, value, _is_httponly) in rows.flatten() {
        // Build a Set-Cookie header string
        let cookie_str = format!(
            "{}={}; Domain={}; Path={}{}",
            name,
            value,
            host,
            path,
            if is_secure != 0 { "; Secure" } else { "" }
        );

        // Parse and insert into cookie store
        let url_str = format!(
            "{}://{}{}",
            if is_secure != 0 { "https" } else { "http" },
            host.trim_start_matches('.'),
            path
        );

        if let Ok(url) = Url::parse(&url_str) {
            if let Ok(cookie) = cookie_store::RawCookie::parse(&cookie_str) {
                let cookie = cookie.into_owned();
                cookie_store.insert_raw(&cookie, &url).ok();
                count += 1;
            }
        }
    }

    // Clean up temp file
    std::fs::remove_file(&temp_path).ok();

    Ok(count)
}
