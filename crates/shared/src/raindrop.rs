use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    #[serde(rename = "_id")]
    pub id: i64,
    pub title: String,
    pub link: String,
    pub tags: Vec<String>,
    pub created: String,
}

#[derive(Debug, Deserialize)]
struct RaindropResponse {
    items: Vec<Bookmark>,
    #[serde(default)]
    count: usize,
}

pub struct RaindropClient {
    client: Client,
    api_token: String,
}

impl RaindropClient {
    pub fn new(api_token: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, api_token })
    }

    pub async fn fetch_bookmarks(&self, tag: &str, since: DateTime<Utc>) -> Result<Vec<Bookmark>> {
        let mut all_bookmarks = Vec::new();
        let mut page = 0;
        let per_page = 50;

        let date_str = since.format("%Y-%m-%d").to_string();
        let search_query = format!("{} created:>{}", tag, date_str);

        loop {
            let url = format!(
                "https://api.raindrop.io/rest/v1/raindrops/0?perpage={}&page={}&search={}",
                per_page,
                page,
                urlencoding::encode(&search_query)
            );

            let response = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.api_token))
                .send()
                .await
                .context("Failed to fetch bookmarks from Raindrop.io")?;

            let status = response.status();
            if !status.is_success() {
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| String::from("unknown error"));
                anyhow::bail!("Raindrop API returned error: {} - {}", status, error_text);
            }

            let raindrop_response = response
                .json::<RaindropResponse>()
                .await
                .context("Failed to parse Raindrop API response")?;

            if raindrop_response.items.is_empty() {
                break;
            }

            all_bookmarks.extend(raindrop_response.items);

            page += 1;

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        Ok(all_bookmarks)
    }
}
