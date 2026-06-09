use anyhow::{Context, Result};
use cookie_store::CookieStore;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
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

/// Open a SQLite database read-only and immutable.
///
/// `immutable=1` tells SQLite to assume the file cannot change, so it reads a
/// database that is locked by a running browser (WAL mode) without taking any
/// lock — and without us copying the entire cookie jar to a predictable, shared
/// /tmp path (which could leak or persist on a crash).
fn open_cookie_db(db_path: &Path) -> Result<Connection> {
    use rusqlite::OpenFlags;

    let uri = Url::from_file_path(db_path)
        .map(|u| format!("{u}?immutable=1"))
        .map_err(|_| anyhow::anyhow!("cookie database path is not absolute: {db_path:?}"))?;

    Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .context("Failed to open cookies database read-only")
}

fn load_firefox_cookies_from_db(
    db_path: &Path,
    cookie_store: &mut CookieStore,
) -> Result<usize> {
    let conn = open_cookie_db(db_path)?;

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

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_open_cookie_db_is_read_only_and_does_not_copy() {
        let dir = std::env::temp_dir().join(format!("briefing-cookie-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("cookies.sqlite");

        // Build a source cookie database.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE moz_cookies (host TEXT, value TEXT);
                 INSERT INTO moz_cookies VALUES ('example.com', 'secret');",
            )
            .unwrap();
        }

        let conn = open_cookie_db(&db_path).unwrap();

        // It reads the real database in place (no copy needed).
        let host: String = conn
            .query_row("SELECT host FROM moz_cookies", [], |r| r.get(0))
            .unwrap();
        assert_eq!(host, "example.com");

        // It must be opened read-only so we can never mutate the user's cookie jar.
        let write = conn.execute("INSERT INTO moz_cookies VALUES ('x', 'y')", []);
        assert!(write.is_err(), "cookie database must be opened read-only");

        std::fs::remove_dir_all(&dir).ok();
    }
}
