use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub struct ArticleContent {
    pub text: String,
    pub published_date: Option<String>,
}

pub struct ContentExtractor {
    client: Client,
    semaphore: Arc<Semaphore>,
}

impl ContentExtractor {
    pub fn new() -> Result<Self> {
        // Create reqwest cookie jar
        let cookie_jar = Arc::new(reqwest::cookie::Jar::default());

        // Load browser cookies for accessing paywalled sites
        if let Ok(chrome_cookies) = crate::cookies::load_chrome_cookies() {
            for cookie in chrome_cookies.iter_any() {
                if let Some(domain) = cookie.domain() {
                    let url_str = format!("https://{}", domain);
                    if let Ok(url) = url::Url::parse(&url_str) {
                        let cookie_str = format!("{}={}", cookie.name(), cookie.value());
                        cookie_jar.add_cookie_str(&cookie_str, &url);
                    }
                }
            }
        }

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .cookie_provider(cookie_jar)
            .build()
            .context("Failed to create HTTP client")?;

        let semaphore = Arc::new(Semaphore::new(10));

        Ok(Self { client, semaphore })
    }

    pub async fn fetch_article_content(&self, url: &str) -> Result<Option<ArticleContent>> {
        let _permit = self.semaphore.acquire().await?;

        for attempt in 0..3 {
            match self.try_fetch_article(url).await {
                Ok(content) => return Ok(content),
                Err(e) => {
                    if attempt == 2 {
                        eprintln!("Failed to fetch {}: {}", url, e);
                        return Ok(None);
                    }
                    let backoff = std::time::Duration::from_millis(500 * (2_u64.pow(attempt)));
                    tokio::time::sleep(backoff).await;
                }
            }
        }

        Ok(None)
    }

    async fn try_fetch_article(&self, url: &str) -> Result<Option<ArticleContent>> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Failed to send HTTP request")?;

        let status = response.status();

        // Provide specific error messages for common HTTP status codes
        match status.as_u16() {
            401 => anyhow::bail!("Access denied (401 Unauthorized) - requires login"),
            403 => anyhow::bail!(
                "Access forbidden (403 Forbidden) - may be paywalled or blocking bots"
            ),
            404 => anyhow::bail!("Page not found (404) - article may have been removed"),
            429 => anyhow::bail!("Rate limited (429) - too many requests"),
            500..=599 => anyhow::bail!("Server error ({}) - website is having issues", status),
            _ if !status.is_success() => anyhow::bail!("HTTP error: {}", status),
            _ => {}
        }

        let html = response
            .text()
            .await
            .context("Failed to read response body")?;

        // Extract publication date from HTML meta tags
        let published_date = self.extract_published_date(&html);

        // Convert HTML to text
        let text = html2text::from_read(html.as_bytes(), 100);

        if text.trim().is_empty() {
            anyhow::bail!("No text content extracted - may require JavaScript or login");
        }

        if text.len() < 100 {
            anyhow::bail!(
                "Content too short ({} chars) - may be paywalled or blocked",
                text.len()
            );
        }

        Ok(Some(ArticleContent {
            text,
            published_date,
        }))
    }

    fn extract_published_date(&self, html: &str) -> Option<String> {
        let document = Html::parse_document(html);

        // Try various meta tag selectors for publication date
        let meta_selectors = vec![
            r#"meta[property="article:published_time"]"#,
            r#"meta[property="og:published_time"]"#,
            r#"meta[name="article:published_time"]"#,
            r#"meta[name="publishdate"]"#,
            r#"meta[name="publish_date"]"#,
            r#"meta[name="date"]"#,
            r#"meta[name="publication_date"]"#,
            r#"meta[itemprop="datePublished"]"#,
            r#"time[datetime]"#,
        ];

        for selector_str in meta_selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                if let Some(element) = document.select(&selector).next() {
                    // Try to get content attribute first (for meta tags)
                    if let Some(content) = element.value().attr("content") {
                        if let Some(formatted) = self.format_date(content) {
                            return Some(formatted);
                        }
                    }
                    // Try datetime attribute (for time tags)
                    if let Some(datetime) = element.value().attr("datetime") {
                        if let Some(formatted) = self.format_date(datetime) {
                            return Some(formatted);
                        }
                    }
                }
            }
        }

        None
    }

    fn format_date(&self, date_str: &str) -> Option<String> {
        // Try parsing ISO 8601 format first
        if let Ok(dt) = date_str.parse::<DateTime<Utc>>() {
            return Some(dt.format("%a, %-d %b %Y").to_string());
        }

        // If it's just a date without time, try parsing that
        if let Ok(naive_date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            let datetime = naive_date.and_hms_opt(0, 0, 0)?;
            let dt: DateTime<Utc> = DateTime::from_naive_utc_and_offset(datetime, Utc);
            return Some(dt.format("%a, %-d %b %Y").to_string());
        }

        None
    }

    pub async fn fetch_articles_parallel(
        &self,
        urls: Vec<String>,
    ) -> Vec<(String, Option<ArticleContent>)> {
        stream::iter(urls)
            .map(|url| {
                let url_clone = url.clone();
                async move {
                    let content = self.fetch_article_content(&url).await.ok().flatten();
                    (url_clone, content)
                }
            })
            .buffer_unordered(10)
            .collect()
            .await
    }
}
