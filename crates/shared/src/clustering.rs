use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::summarizer::Summary;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Story {
    pub title: String,
    pub url: String,
    pub created: String,
    pub summary: Summary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topic {
    pub title: String,
    pub stories: Vec<Story>,
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

#[derive(Deserialize)]
struct ClusteringResult {
    topics: Vec<TopicCluster>,
}

#[derive(Deserialize)]
struct TopicCluster {
    title: String,
    article_indices: Vec<usize>,
}

pub struct TopicClusterer {
    client: Client,
    api_key: String,
}

impl TopicClusterer {
    pub fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, api_key })
    }

    pub async fn cluster_stories(&self, stories: Vec<Story>) -> Result<Vec<Topic>> {
        if stories.is_empty() {
            return Ok(Vec::new());
        }

        if stories.len() == 1 {
            return Ok(vec![Topic {
                title: "News".to_string(),
                stories,
            }]);
        }

        // Retry logic with exponential backoff for rate limits
        for attempt in 0..5 {
            match self.try_cluster_with_ai(&stories).await {
                Ok(topics) => return Ok(topics),
                Err(e) => {
                    let error_msg = e.to_string();
                    let is_rate_limit =
                        error_msg.contains("rate_limit") || error_msg.contains("429");

                    if attempt == 4 {
                        eprintln!(
                            "Clustering failed after {} attempts: {}, using chronological fallback",
                            attempt + 1,
                            e
                        );
                        return Ok(self.fallback_chronological(stories));
                    }

                    // Longer backoff for rate limits
                    let backoff = if is_rate_limit {
                        std::time::Duration::from_secs(15 * (attempt + 1) as u64)
                    } else {
                        std::time::Duration::from_millis(1000 * (2_u64.pow(attempt as u32)))
                    };

                    if is_rate_limit {
                        eprintln!("Rate limit hit during clustering, waiting {:?} before retry {} of 5...", backoff, attempt + 2);
                    } else {
                        eprintln!(
                            "Clustering error (attempt {} of 5): {}, retrying after {:?}...",
                            attempt + 1,
                            e,
                            backoff
                        );
                    }

                    tokio::time::sleep(backoff).await;
                }
            }
        }

        // This should never be reached due to the attempt == 4 check above, but keeping for safety
        Ok(self.fallback_chronological(stories))
    }

    async fn try_cluster_with_ai(&self, stories: &[Story]) -> Result<Vec<Topic>> {
        let articles_text = stories
            .iter()
            .enumerate()
            .map(|(idx, story)| {
                let first_point = match &story.summary {
                    Summary::Editorial { lede, .. } => lede.as_str(),
                    Summary::Product { the_product, .. } => the_product.as_str(),
                    _ => "",
                };
                format!("{}: {} - {}", idx, story.title, first_point)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            r#"You are analyzing a list of news articles for a tech podcast briefing.

GROUPING RULES (in priority order):
1. PRIMARY: If an article is primarily about a specific company (Google, Apple, Microsoft, Tesla, Meta, Amazon, etc.), use the company name as the topic title
2. Group all articles about the same company together under that company's name
3. For articles not primarily about a single company, use a descriptive topic (e.g., "AI Development", "Privacy & Security", "Industry News")
4. Use concise topic names (1-3 words preferred, company names exactly as they are commonly known)

Articles:
{}

Format your response as JSON:
{{
  "topics": [
    {{
      "title": "Apple",
      "article_indices": [0, 3, 7]
    }},
    {{
      "title": "Google",
      "article_indices": [1, 5]
    }},
    {{
      "title": "AI Development",
      "article_indices": [2, 4, 6]
    }}
  ]
}}

Important: Every article index from 0 to {} must appear in exactly one topic."#,
            articles_text,
            stories.len() - 1
        );

        let request = ClaudeRequest {
            model: "claude-haiku-4-5-20251001".to_string(),
            max_tokens: 2048,
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

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            anyhow::bail!(
                "Claude API error (status {}): {}",
                status.as_u16(),
                error_text
            );
        }

        let claude_response = response
            .json::<ClaudeResponse>()
            .await
            .context("Failed to parse Claude API response")?;

        let response_text = claude_response
            .content
            .first()
            .map(|c| c.text.as_str())
            .unwrap_or("");

        let json_text = if let Some(start) = response_text.find('{') {
            if let Some(end) = response_text.rfind('}') {
                &response_text[start..=end]
            } else {
                response_text
            }
        } else {
            response_text
        };

        let clustering_result: ClusteringResult =
            serde_json::from_str(json_text).context("Failed to parse clustering JSON response")?;

        let mut topics = Vec::new();
        for cluster in clustering_result.topics {
            let mut topic_stories = Vec::new();
            for &idx in &cluster.article_indices {
                if idx < stories.len() {
                    topic_stories.push(stories[idx].clone());
                }
            }
            // Sort stories oldest-first so the org file starts in chronological order
            topic_stories.sort_by(|a, b| {
                let date_a = parse_date_for_sorting(&a.created);
                let date_b = parse_date_for_sorting(&b.created);
                match (date_a, date_b) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            });
            if !topic_stories.is_empty() {
                topics.push(Topic {
                    title: cluster.title,
                    stories: topic_stories,
                });
            }
        }

        if topics.is_empty() {
            anyhow::bail!("No topics generated from clustering");
        }

        Ok(topics)
    }

    fn fallback_chronological(&self, stories: Vec<Story>) -> Vec<Topic> {
        vec![Topic {
            title: "News Stories".to_string(),
            stories,
        }]
    }
}

/// Parse a date string for sorting. Handles RFC 3339 and common date-only formats.
fn parse_date_for_sorting(date_str: &str) -> Option<DateTime<FixedOffset>> {
    if date_str.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
        return Some(dt);
    }
    for fmt in &["%a, %e %b %Y", "%a, %d %b %Y", "%e %b %Y", "%d %b %Y", "%Y-%m-%d"] {
        if let Ok(nd) = NaiveDate::parse_from_str(date_str.trim(), fmt) {
            return nd
                .and_hms_opt(0, 0, 0)
                .map(|ndt| ndt.and_utc().fixed_offset());
        }
    }
    None
}
