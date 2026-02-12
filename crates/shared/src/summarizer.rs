use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Summary {
    Editorial {
        whats_happening: String,
        why_it_matters: String,
        big_picture: String,
        quote: Option<String>,
    },
    Product {
        the_product: String,
        cost: String,
        availability: String,
        platforms: String,
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
            r#"You are a journalist writing in Axios Smart Brevity style. Summarize the article below using the appropriate format.

First, determine: Is this article primarily about a specific PRODUCT (hardware, software, app, device) or is it EDITORIAL (news, policy, analysis, industry event)?

RULES:
1. Use ONLY information from the article - no external knowledge
2. Each section should be 1-2 concise sentences
3. If the article has insufficient content, respond with: "Insufficient content for summary"
4. If there are direct quotes with clear speaker attribution, include the most important one

If EDITORIAL, respond in this exact format:
FORMAT: EDITORIAL
WHATS_HAPPENING: One strong sentence capturing the core news or development.
WHY_IT_MATTERS: 1-2 sentences explaining why this is significant.
BIG_PICTURE: One sentence on broader industry or societal implications. Omit this line if the article is too narrow for broader context.
QUOTE: "quote text" -- Speaker Name

If PRODUCT, respond in this exact format:
FORMAT: PRODUCT
THE_PRODUCT: What the product is and what it does (1-2 sentences).
COST: Pricing details. Omit this line if pricing is not mentioned.
AVAILABILITY: When and where it is available. Omit this line if not mentioned.
PLATFORMS: What platforms or operating systems it runs on. Omit this line for hardware-only products or if not mentioned.
QUOTE: "quote text" -- Speaker Name

Omit the QUOTE line if there are no quotes or no clear speaker attribution in the article.

Article:
{}"#,
            truncated_content
        );

        let request = ClaudeRequest {
            model: "claude-3-5-haiku-20241022".to_string(),
            max_tokens: 768,
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

        self.parse_smart_brevity(summary_text)
    }

    fn parse_smart_brevity(&self, text: &str) -> Result<Summary> {
        let mut format_type = None;
        let mut quote = None;
        let mut whats_happening = String::new();
        let mut why_it_matters = String::new();
        let mut big_picture = String::new();
        let mut the_product = String::new();
        let mut cost = String::new();
        let mut availability = String::new();
        let mut platforms = String::new();

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some(fmt) = trimmed.strip_prefix("FORMAT:") {
                format_type = Some(fmt.trim().to_uppercase());
            } else if let Some(val) = trimmed.strip_prefix("WHATS_HAPPENING:") {
                whats_happening = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("WHY_IT_MATTERS:") {
                why_it_matters = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("BIG_PICTURE:") {
                big_picture = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("THE_PRODUCT:") {
                the_product = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("COST:") {
                cost = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("AVAILABILITY:") {
                availability = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("PLATFORMS:") {
                platforms = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("QUOTE:") {
                let val = val.trim();
                if !val.is_empty() {
                    quote = Some(val.to_string());
                }
            }
        }

        // Auto-detect format from content if FORMAT: line is missing
        let is_product = match format_type.as_deref() {
            Some("PRODUCT") => true,
            Some("EDITORIAL") => false,
            _ => !the_product.is_empty(),
        };

        if is_product {
            if the_product.is_empty() {
                return Ok(Summary::Failed(
                    "Product format missing THE_PRODUCT field".to_string(),
                ));
            }
            Ok(Summary::Product {
                the_product,
                cost,
                availability,
                platforms,
                quote,
            })
        } else {
            if whats_happening.is_empty() || why_it_matters.is_empty() {
                return Ok(Summary::Failed(
                    "Editorial format missing required fields".to_string(),
                ));
            }
            Ok(Summary::Editorial {
                whats_happening,
                why_it_matters,
                big_picture,
                quote,
            })
        }
    }

    pub async fn summarize_articles_parallel(
        &self,
        articles: Vec<(String, String)>,
    ) -> Vec<(String, Summary)> {
        let results: Vec<(String, Summary)> = stream::iter(articles)
            .map(|(url, content)| {
                let url_clone = url.clone();
                async move {
                    let summary = self
                        .summarize_article(&content)
                        .await
                        .unwrap_or_else(|e| Summary::Failed(e.to_string()));
                    // Print progress dot
                    eprint!(".");
                    let _ = std::io::stderr().flush();
                    (url_clone, summary)
                }
            })
            .buffer_unordered(2) // Reduced to 2 to avoid rate limits
            .collect()
            .await;
        eprintln!(); // Newline after dots
        results
    }
}
