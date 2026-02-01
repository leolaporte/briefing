use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Summary {
    Success {
        points: Vec<String>,
        quote: Option<String>,
    },
    Insufficient,
    Failed(String),
}

#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<Content>,
}

#[derive(Deserialize)]
struct Content {
    text: String,
}

pub struct ClaudeSummarizer {
    client: Client,
    api_key: String,
    semaphore: Arc<Semaphore>,
}

impl ClaudeSummarizer {
    pub fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("Failed to create HTTP client")?;

        // Reduce concurrency to avoid rate limits (50k tokens/min)
        let semaphore = Arc::new(Semaphore::new(2));

        Ok(Self {
            client,
            api_key,
            semaphore,
        })
    }

    pub async fn summarize_article(&self, content: &str) -> Result<Summary> {
        let _permit = self.semaphore.acquire().await?;

        for attempt in 0..5 {
            match self.try_summarize(content).await {
                Ok(summary) => {
                    // Add small delay after successful request to spread load
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    return Ok(summary);
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let is_rate_limit = error_msg.contains("rate_limit");

                    if attempt == 4 {
                        eprintln!("Failed to summarize: {}", e);
                        return Ok(Summary::Failed(e.to_string()));
                    }

                    // Longer backoff for rate limits
                    let backoff = if is_rate_limit {
                        std::time::Duration::from_secs(15 * (attempt + 1) as u64)
                    } else {
                        std::time::Duration::from_millis(1000 * (2_u64.pow(attempt as u32)))
                    };

                    if is_rate_limit {
                        eprintln!("Rate limit hit, waiting {:?} before retry...", backoff);
                    }

                    tokio::time::sleep(backoff).await;
                }
            }
        }

        Ok(Summary::Failed("Max retries reached".to_string()))
    }

    async fn try_summarize(&self, content: &str) -> Result<Summary> {
        // Truncate content to 10000 chars, respecting UTF-8 boundaries
        let truncated_content = if content.len() > 10000 {
            let mut end = 10000;
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            &content[..end]
        } else {
            content
        };

        let prompt = format!(
            r#"You are a text summarization specialist. Extract exactly 5 key points from the article below, and if there are any direct quotes, extract the most important one with attribution.

RULES:
1. Each point must be under 20 words
2. Use ONLY text from the article - no external knowledge
3. Each point must be supported by specific article content
4. If fewer than 5 valid points exist, respond with: "Insufficient content for summary"
5. Format: Bullet points using dashes (-)
6. Use only factual statements from the article text
7. If there are direct quotes in the article, select the most important one (often the first quote, but use your judgment)
8. The quote should be on a line starting with "QUOTE: " followed by the quote text in quotation marks and attribution
9. Format for quotes: QUOTE: "quote text" -- Speaker Name

Article:
{}

Format your response as:
QUOTE: "the most important quote if one exists" -- Speaker Name
- First key point
- Second key point
- Third key point
- Fourth key point
- Fifth key point

If there are no quotes in the article, omit the QUOTE line entirely.
If there's a quote but no clear speaker attribution in the article, omit the QUOTE line."#,
            truncated_content
        );

        let request = ClaudeRequest {
            model: "claude-3-5-haiku-20241022".to_string(),
            max_tokens: 512,
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
            }],
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Claude API")?;

        if !response.status().is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            anyhow::bail!("Claude API error: {}", error_text);
        }

        let claude_response = response
            .json::<ClaudeResponse>()
            .await
            .context("Failed to parse Claude API response")?;

        let summary_text = claude_response
            .content
            .first()
            .map(|c| c.text.as_str())
            .unwrap_or("");

        if summary_text.contains("Insufficient content for summary") {
            return Ok(Summary::Insufficient);
        }

        let (quote, bullets) = self.parse_summary_with_quote(summary_text);

        if bullets.len() == 5 {
            Ok(Summary::Success {
                points: bullets,
                quote,
            })
        } else {
            Ok(Summary::Failed(format!(
                "Expected 5 bullets, got {}",
                bullets.len()
            )))
        }
    }

    fn parse_summary_with_quote(&self, text: &str) -> (Option<String>, Vec<String>) {
        let mut quote = None;
        let mut bullets = Vec::new();

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Check for quote line
            if trimmed.starts_with("QUOTE:") {
                let quote_text = trimmed.strip_prefix("QUOTE:").unwrap().trim();
                // Keep the quote as-is (it already includes quotes and attribution)
                if !quote_text.is_empty() {
                    quote = Some(quote_text.to_string());
                }
                continue;
            }

            // Check for bullet points
            if let Some(stripped) = trimmed.strip_prefix(|c: char| c.is_numeric()) {
                let stripped = stripped
                    .trim_start_matches(|c: char| c == '.' || c == ')' || c.is_whitespace());
                if !stripped.is_empty() {
                    bullets.push(stripped.to_string());
                }
                continue;
            }

            if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with('•') {
                let stripped = trimmed[1..].trim();
                if !stripped.is_empty() {
                    bullets.push(stripped.to_string());
                }
            }
        }

        (quote, bullets)
    }

    #[allow(dead_code)]
    fn parse_bullet_points(&self, text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                if let Some(stripped) = trimmed.strip_prefix(|c: char| c.is_numeric()) {
                    let stripped = stripped
                        .trim_start_matches(|c: char| c == '.' || c == ')' || c.is_whitespace());
                    if !stripped.is_empty() {
                        return Some(stripped.to_string());
                    }
                }
                if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with('•')
                {
                    let stripped = trimmed[1..].trim();
                    if !stripped.is_empty() {
                        return Some(stripped.to_string());
                    }
                }
                None
            })
            .collect()
    }

    pub async fn summarize_articles_parallel(
        &self,
        articles: Vec<(String, String)>,
    ) -> Vec<(String, Summary)> {
        stream::iter(articles)
            .map(|(url, content)| {
                let url_clone = url.clone();
                async move {
                    let summary = self
                        .summarize_article(&content)
                        .await
                        .unwrap_or_else(|e| Summary::Failed(e.to_string()));
                    (url_clone, summary)
                }
            })
            .buffer_unordered(2) // Reduced to 2 to avoid rate limits
            .collect()
            .await
    }
}
