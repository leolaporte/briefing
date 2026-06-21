use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

use crate::summarizer::Summary;

const GLM_MODEL: &str = "glm-5.2";
const ZAI_API_URL: &str = "https://api.z.ai/api/anthropic/v1/messages";
const CLUSTER_TIMEOUT: Duration = Duration::from_secs(90);

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
    pub fn new() -> Result<Self> {
        // The API key lives in the env var named by BRIEFING_LLM_KEY_ENV
        // (default ZAI_API_KEY), so a run can target a different backend —
        // e.g. Anthropic via ANTHROPIC_API_KEY — without code changes.
        let key_var = std::env::var("BRIEFING_LLM_KEY_ENV")
            .unwrap_or_else(|_| "ZAI_API_KEY".to_string());
        let api_key = std::env::var(&key_var)
            .with_context(|| format!("{key_var} not set"))?;
        let client = Client::builder()
            .timeout(CLUSTER_TIMEOUT)
            .build()
            .context("Failed to build HTTP client")?;
        Ok(TopicClusterer { client, api_key })
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

                    // Auth errors are fatal — don't retry
                    if error_msg.contains("authentication_error")
                        || error_msg.contains("invalid_api_key")
                        || error_msg.contains("401")
                    {
                        anyhow::bail!("Authentication failed: {}", error_msg);
                    }

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

        // Endpoint/model are env-overridable for testing alternate backends
        // (e.g. a local llama.cpp /v1/messages server). Defaults to z.ai GLM.
        let model = std::env::var("BRIEFING_LLM_MODEL").unwrap_or_else(|_| GLM_MODEL.to_string());
        let url = std::env::var("BRIEFING_LLM_URL").unwrap_or_else(|_| ZAI_API_URL.to_string());

        let body = json!({
            "model": model,
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": prompt}]
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", &self.api_key))
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Clustering API request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status == 401 {
                anyhow::bail!("authentication_error: {}", text);
            }
            anyhow::bail!("API error {}: {}", status, text);
        }

        let data: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse clustering API response")?;

        let response_text = data["content"][0]["text"]
            .as_str()
            .context("No text in clustering API response")?;

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
        let mut assigned = vec![false; stories.len()];
        for cluster in clustering_result.topics {
            let mut topic_stories = Vec::new();
            for &idx in &cluster.article_indices {
                // Skip out-of-range indices and indices the model listed twice
                if idx < stories.len() && !assigned[idx] {
                    assigned[idx] = true;
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

        // The model is told to assign every index, but it can omit some; don't
        // silently drop those stories from the briefing.
        let unassigned: Vec<Story> = assigned
            .iter()
            .enumerate()
            .filter(|(_, done)| !**done)
            .map(|(idx, _)| stories[idx].clone())
            .collect();
        if !unassigned.is_empty() {
            eprintln!(
                "Clustering left {} story(ies) unassigned, adding them to \"More News\"",
                unassigned.len()
            );
            topics.push(Topic {
                title: "More News".to_string(),
                stories: unassigned,
            });
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
    for fmt in &[
        "%a, %e %b %Y",
        "%a, %d %b %Y",
        "%e %b %Y",
        "%d %b %Y",
        "%Y-%m-%d",
    ] {
        if let Ok(nd) = NaiveDate::parse_from_str(date_str.trim(), fmt) {
            return nd
                .and_hms_opt(0, 0, 0)
                .map(|ndt| ndt.and_utc().fixed_offset());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Story / Topic Construction ====================

    fn make_story(title: &str, url: &str, created: &str) -> Story {
        Story {
            title: title.to_string(),
            url: url.to_string(),
            created: created.to_string(),
            summary: Summary::Editorial {
                lede: "Test lede".to_string(),
                nutgraf: "Test nutgraf".to_string(),
                quote: None,
            },
        }
    }

    // ==================== parse_date_for_sorting ====================

    #[test]
    fn test_parse_date_rfc3339() {
        let date = parse_date_for_sorting("2026-02-01T12:00:00+00:00");
        assert!(date.is_some());
    }

    #[test]
    fn test_parse_date_iso8601() {
        let date = parse_date_for_sorting("2026-02-01");
        assert!(date.is_some());
    }

    #[test]
    fn test_parse_date_day_month_year_zero_padded() {
        // %d is zero-padded
        let date = parse_date_for_sorting("01 Feb 2026");
        assert!(date.is_some());
    }

    #[test]
    fn test_parse_date_day_month_year_double_digit() {
        // Double-digit day works with both %e and %d
        let date = parse_date_for_sorting("15 Feb 2026");
        assert!(date.is_some());
    }

    #[test]
    fn test_parse_date_empty_string() {
        let date = parse_date_for_sorting("");
        assert!(date.is_none());
    }

    #[test]
    fn test_parse_date_invalid() {
        let date = parse_date_for_sorting("not a date");
        assert!(date.is_none());
    }

    #[test]
    fn test_parse_date_ordering() {
        let older = parse_date_for_sorting("2026-01-01").unwrap();
        let newer = parse_date_for_sorting("2026-12-31").unwrap();
        assert!(older < newer);
    }

    #[test]
    fn test_parse_date_rfc3339_ordering() {
        let older = parse_date_for_sorting("2026-01-15T08:00:00+00:00").unwrap();
        let newer = parse_date_for_sorting("2026-01-15T20:00:00+00:00").unwrap();
        assert!(older < newer);
    }

    // ==================== TopicClusterer edge cases ====================

    fn make_clusterer() -> TopicClusterer {
        TopicClusterer {
            client: reqwest::Client::new(),
            api_key: "test".to_string(),
        }
    }

    #[test]
    fn test_topic_clusterer_fallback_chronological() {
        let clusterer = make_clusterer();
        let stories = vec![
            make_story("A", "https://a.com", "2026-01-01"),
            make_story("B", "https://b.com", "2026-01-02"),
        ];

        let topics = clusterer.fallback_chronological(stories.clone());
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].title, "News Stories");
        assert_eq!(topics[0].stories.len(), 2);
    }
}
