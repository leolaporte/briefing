use serde::{Deserialize, Serialize};

use crate::clustering::Topic;

/// Metadata about the show
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowInfo {
    pub name: String,
    pub slug: String,
    pub tag: String,
}

impl ShowInfo {
    pub fn new(name: impl Into<String>, slug: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            slug: slug.into(),
            tag: tag.into(),
        }
    }
}

/// Complete briefing data for serialization
#[derive(Debug, Serialize, Deserialize)]
pub struct BriefingData {
    pub version: String,
    pub created_at: String,
    pub show: ShowInfo,
    pub topics: Vec<Topic>,
}

impl BriefingData {
    pub fn new(show: ShowInfo, topics: Vec<Topic>) -> Self {
        Self {
            version: "1.0".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            show,
            topics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::Summary;

    #[test]
    fn test_show_info_new() {
        let show = ShowInfo::new("This Week in Tech", "twit", "TWiT");
        assert_eq!(show.name, "This Week in Tech");
        assert_eq!(show.slug, "twit");
        assert_eq!(show.tag, "TWiT");
    }

    #[test]
    fn test_show_info_from_string_types() {
        let show = ShowInfo::new(
            String::from("MacBreak Weekly"),
            String::from("mbw"),
            String::from("MBW"),
        );
        assert_eq!(show.name, "MacBreak Weekly");
    }

    #[test]
    fn test_briefing_data_new() {
        let show = ShowInfo::new("Test Show", "test", "TEST");
        let topics = vec![Topic {
            title: "Tech News".to_string(),
            stories: vec![],
        }];

        let data = BriefingData::new(show.clone(), topics);

        assert_eq!(data.version, "1.0");
        assert_eq!(data.show.name, "Test Show");
        assert_eq!(data.topics.len(), 1);
        // created_at should be a valid RFC3339 timestamp
        assert!(data.created_at.contains("T"));
    }

    #[test]
    fn test_briefing_data_serialization() {
        let show = ShowInfo::new("Test", "test", "TEST");
        let story = crate::clustering::Story {
            title: "Test Article".to_string(),
            url: "https://example.com".to_string(),
            created: "2026-02-01".to_string(),
            summary: Summary::Success {
                points: vec!["Point 1".to_string()],
                quote: None,
            },
        };
        let topics = vec![Topic {
            title: "News".to_string(),
            stories: vec![story],
        }];
        let data = BriefingData::new(show, topics);

        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("Test Article"));
        assert!(json.contains("https://example.com"));
        assert!(json.contains("Point 1"));
    }

    #[test]
    fn test_briefing_data_deserialization() {
        let json = r#"{
            "version": "1.0",
            "created_at": "2026-02-01T00:00:00Z",
            "show": {"name": "TWiT", "slug": "twit", "tag": "TWiT"},
            "topics": [
                {
                    "title": "Apple",
                    "stories": [{
                        "title": "Test",
                        "url": "https://test.com",
                        "created": "2026-02-01",
                        "summary": {"Success": {"points": ["A", "B"], "quote": null}}
                    }]
                }
            ]
        }"#;

        let data: BriefingData = serde_json::from_str(json).unwrap();
        assert_eq!(data.version, "1.0");
        assert_eq!(data.show.name, "TWiT");
        assert_eq!(data.topics.len(), 1);
        assert_eq!(data.topics[0].stories.len(), 1);
    }
}
