use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Semaphore;

fn is_auth_error(msg: &str) -> bool {
    msg.contains("authentication_error") || msg.contains("invalid x-api-key")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Summary {
    Editorial {
        lede: String,
        nutgraf: String,
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
    system: Vec<SystemBlock>,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct SystemBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
    cache_control: CacheControl,
}

#[derive(Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    control_type: String,
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

const SUMMARIZER_SYSTEM_PROMPT: &str = r#"You are a journalist summarizing articles using the nut graph structure. Summarize the article below using the appropriate format.

First, determine: Is this article primarily about a specific PRODUCT (hardware, software, app, device) or is it EDITORIAL (news, policy, analysis, industry event)?

RULES:
1. Use ONLY information from the article - no external knowledge
2. If the article has insufficient content, respond with: "Insufficient content for summary"
3. QUOTE must be copied VERBATIM from the article — the exact words as they appear, with clear speaker attribution. Do not paraphrase or alter the quote in any way.

If EDITORIAL, respond in this exact format:
FORMAT: EDITORIAL
QUOTE: "exact verbatim quote from the article" -- Speaker Name
LEDE: One strong sentence identifying WHO is involved and WHAT happened or was announced.
NUTGRAF: A paragraph (2-4 sentences) explaining WHY this matters. Contextualize the most important facts and give the reader a clear understanding of the central issue or topic.

If PRODUCT, respond in this exact format:
FORMAT: PRODUCT
THE_PRODUCT: What the product is and what it does (1-2 sentences).
COST: Pricing details. Omit this line if pricing is not mentioned.
AVAILABILITY: When and where it is available. Omit this line if not mentioned.
PLATFORMS: What platforms or operating systems it runs on. Omit this line for hardware-only products or if not mentioned.
QUOTE: "exact verbatim quote from the article" -- Speaker Name

Omit the QUOTE line if there are no direct quotes with clear speaker attribution in the article."#;

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

                    // Auth errors are fatal — don't retry, propagate immediately
                    if is_auth_error(&error_msg) {
                        anyhow::bail!("Authentication failed: {}", error_msg);
                    }

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

        let request = ClaudeRequest {
            model: "claude-haiku-4-5-20251001".to_string(),
            max_tokens: 768,
            system: vec![SystemBlock {
                block_type: "text".to_string(),
                text: SUMMARIZER_SYSTEM_PROMPT.to_string(),
                cache_control: CacheControl {
                    control_type: "ephemeral".to_string(),
                },
            }],
            messages: vec![Message {
                role: "user".to_string(),
                content: format!("Article:\n{}", truncated_content),
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
        let mut lede = String::new();
        let mut nutgraf = String::new();
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
            } else if let Some(val) = trimmed.strip_prefix("LEDE:") {
                lede = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("NUTGRAF:") {
                nutgraf = val.trim().to_string();
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
            if lede.is_empty() || nutgraf.is_empty() {
                return Ok(Summary::Failed(
                    "Editorial format missing required fields".to_string(),
                ));
            }
            Ok(Summary::Editorial {
                lede,
                nutgraf,
                quote,
            })
        }
    }

    pub async fn summarize_articles_parallel(
        &self,
        articles: Vec<(String, String)>,
    ) -> Result<Vec<(String, Summary)>> {
        let auth_failed = Arc::new(AtomicBool::new(false));

        let results: Vec<(String, Summary)> = stream::iter(articles)
            .map(|(url, content)| {
                let url_clone = url.clone();
                let auth_failed = auth_failed.clone();
                async move {
                    if auth_failed.load(Ordering::Relaxed) {
                        return (url_clone, Summary::Failed("Skipped — authentication failed".to_string()));
                    }

                    let summary = match self.summarize_article(&content).await {
                        Ok(summary) => summary,
                        Err(e) => {
                            let msg = e.to_string();
                            if is_auth_error(&msg) {
                                auth_failed.store(true, Ordering::Relaxed);
                            }
                            Summary::Failed(msg)
                        }
                    };
                    eprint!(".");
                    let _ = std::io::stderr().flush();
                    (url_clone, summary)
                }
            })
            .buffer_unordered(2)
            .collect()
            .await;
        eprintln!();

        if auth_failed.load(Ordering::Relaxed) {
            anyhow::bail!("API authentication failed — check your CLAUDE_API_KEY");
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summarizer() -> ClaudeSummarizer {
        ClaudeSummarizer::new("fake-key".to_string()).unwrap()
    }

    // ==================== is_auth_error ====================

    #[test]
    fn test_is_auth_error_authentication_error() {
        assert!(is_auth_error("authentication_error: invalid key"));
    }

    #[test]
    fn test_is_auth_error_invalid_api_key() {
        assert!(is_auth_error("invalid x-api-key provided"));
    }

    #[test]
    fn test_is_auth_error_rate_limit() {
        assert!(!is_auth_error("rate_limit exceeded"));
    }

    #[test]
    fn test_is_auth_error_generic() {
        assert!(!is_auth_error("something went wrong"));
    }

    // ==================== parse_smart_brevity — Editorial ====================

    #[test]
    fn test_parse_editorial_full() {
        let s = summarizer();
        let text = "\
FORMAT: EDITORIAL
QUOTE: \"This is huge\" -- John Doe
LEDE: Apple announced a new chip.
NUTGRAF: This matters because performance gains change the industry.";

        let result = s.parse_smart_brevity(text).unwrap();
        match result {
            Summary::Editorial { lede, nutgraf, quote } => {
                assert_eq!(lede, "Apple announced a new chip.");
                assert!(nutgraf.contains("performance gains"));
                assert!(quote.unwrap().contains("This is huge"));
            }
            _ => panic!("Expected Editorial"),
        }
    }

    #[test]
    fn test_parse_editorial_without_quote() {
        let s = summarizer();
        let text = "\
FORMAT: EDITORIAL
LEDE: Something happened.
NUTGRAF: It matters for reasons.";

        let result = s.parse_smart_brevity(text).unwrap();
        match result {
            Summary::Editorial { quote, .. } => assert!(quote.is_none()),
            _ => panic!("Expected Editorial"),
        }
    }

    #[test]
    fn test_parse_editorial_missing_lede() {
        let s = summarizer();
        let text = "\
FORMAT: EDITORIAL
NUTGRAF: It matters.";

        let result = s.parse_smart_brevity(text).unwrap();
        assert!(matches!(result, Summary::Failed(_)));
    }

    #[test]
    fn test_parse_editorial_missing_nutgraf() {
        let s = summarizer();
        let text = "\
FORMAT: EDITORIAL
LEDE: Something happened.";

        let result = s.parse_smart_brevity(text).unwrap();
        assert!(matches!(result, Summary::Failed(_)));
    }

    // ==================== parse_smart_brevity — Product ====================

    #[test]
    fn test_parse_product_full() {
        let s = summarizer();
        let text = "\
FORMAT: PRODUCT
THE_PRODUCT: A new smartwatch with health sensors.
COST: $399.
AVAILABILITY: March 2026.
PLATFORMS: iOS, Android.
QUOTE: \"Best watch ever\" -- Tim Cook";

        let result = s.parse_smart_brevity(text).unwrap();
        match result {
            Summary::Product { the_product, cost, availability, platforms, quote } => {
                assert!(the_product.contains("smartwatch"));
                assert_eq!(cost, "$399.");
                assert!(availability.contains("March"));
                assert!(platforms.contains("iOS"));
                assert!(quote.unwrap().contains("Best watch ever"));
            }
            _ => panic!("Expected Product"),
        }
    }

    #[test]
    fn test_parse_product_optional_fields() {
        let s = summarizer();
        let text = "\
FORMAT: PRODUCT
THE_PRODUCT: A new app for task management.";

        let result = s.parse_smart_brevity(text).unwrap();
        match result {
            Summary::Product { cost, availability, platforms, quote, .. } => {
                assert!(cost.is_empty());
                assert!(availability.is_empty());
                assert!(platforms.is_empty());
                assert!(quote.is_none());
            }
            _ => panic!("Expected Product"),
        }
    }

    #[test]
    fn test_parse_product_missing_the_product() {
        let s = summarizer();
        let text = "\
FORMAT: PRODUCT
COST: $99.";

        let result = s.parse_smart_brevity(text).unwrap();
        assert!(matches!(result, Summary::Failed(_)));
    }

    // ==================== parse_smart_brevity — Auto-detect ====================

    #[test]
    fn test_parse_auto_detects_product_from_the_product_field() {
        let s = summarizer();
        // No FORMAT line, but has THE_PRODUCT — should auto-detect as Product
        let text = "\
THE_PRODUCT: A new laptop with M4 chip.
COST: $1,999.";

        let result = s.parse_smart_brevity(text).unwrap();
        assert!(matches!(result, Summary::Product { .. }));
    }

    #[test]
    fn test_parse_auto_detects_editorial_no_product_field() {
        let s = summarizer();
        // No FORMAT line, no THE_PRODUCT — should default to Editorial if lede/nutgraf present
        let text = "\
LEDE: Apple reported earnings.
NUTGRAF: Revenue beat expectations.";

        let result = s.parse_smart_brevity(text).unwrap();
        assert!(matches!(result, Summary::Editorial { .. }));
    }

    // ==================== parse_smart_brevity — Edge cases ====================

    #[test]
    fn test_parse_empty_string() {
        let s = summarizer();
        let result = s.parse_smart_brevity("").unwrap();
        assert!(matches!(result, Summary::Failed(_)));
    }

    #[test]
    fn test_parse_insufficient_content() {
        let s = summarizer();
        let text = "Insufficient content for summary";
        // This is handled in try_summarize, not parse_smart_brevity directly,
        // but parse should still return Failed for text with no fields
        let result = s.parse_smart_brevity(text).unwrap();
        assert!(matches!(result, Summary::Failed(_)));
    }
}
