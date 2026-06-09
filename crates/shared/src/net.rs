//! Network safety helpers for the extractor.
//!
//! The nightly briefing fetches article bodies from up to 10 sites concurrently.
//! A hostile or misbehaving publisher could stream an unbounded response and
//! exhaust memory, silently killing the 3am run. Reading the body with a hard
//! byte cap prevents that.

/// Maximum number of bytes to read from a single HTTP response body.
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

/// Read an HTTP response body, aborting if it exceeds `max` bytes. Streaming the
/// body chunk-by-chunk bounds memory regardless of what the server claims or sends.
pub async fn read_body_capped(
    mut resp: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if buf.len() + chunk.len() > max {
            return Err(format!("response body exceeded {max}-byte cap"));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_body_over_cap() {
        let resp = reqwest::Response::from(http::Response::new(vec![0u8; 100]));
        assert!(read_body_capped(resp, 50).await.is_err());
    }

    #[tokio::test]
    async fn returns_body_within_cap() {
        let resp = reqwest::Response::from(http::Response::new(vec![9u8; 40]));
        let out = read_body_capped(resp, 50).await.unwrap();
        assert_eq!(out, vec![9u8; 40]);
    }
}
