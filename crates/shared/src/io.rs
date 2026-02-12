use anyhow::{Context, Result};
use chrono::DateTime;
use std::fs;
use std::path::PathBuf;

use crate::models::BriefingData;

/// Get the default directory for storing story files
pub fn get_default_stories_dir() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir()
        .context("Could not determine local data directory")?
        .join("podcast-briefing")
        .join("stories");

    fs::create_dir_all(&data_dir).context("Failed to create stories directory")?;

    Ok(data_dir)
}

/// Save story data to a JSON file
pub fn save_stories(data: &BriefingData, filename: &str) -> Result<PathBuf> {
    let stories_dir = get_default_stories_dir()?;
    let filepath = stories_dir.join(filename);

    let json = serde_json::to_string_pretty(data).context("Failed to serialize briefing data")?;

    fs::write(&filepath, json).context("Failed to write story file")?;

    Ok(filepath)
}

/// Load story data from a JSON file
pub fn load_stories(filepath: &PathBuf) -> Result<BriefingData> {
    // Check if file exists
    if !filepath.exists() {
        anyhow::bail!("Story file not found: {}", filepath.display());
    }

    let content = fs::read_to_string(filepath)
        .with_context(|| format!("Failed to read story file: {}", filepath.display()))?;

    // Try to parse JSON with helpful error message
    let data: BriefingData = serde_json::from_str(&content)
        .with_context(|| {
            format!(
                "Failed to parse story JSON from {}. The file may be corrupted or not a valid story file.",
                filepath.display()
            )
        })?;

    // Validate version
    if data.version != "1.0" {
        anyhow::bail!(
            "Unsupported story file version: {}. Expected 1.0. Please regenerate the story file with collect-stories.",
            data.version
        );
    }

    // Validate required fields
    if data.topics.is_empty() {
        anyhow::bail!(
            "Story file {} contains no topics. The file may be incomplete.",
            filepath.display()
        );
    }

    Ok(data)
}

/// List all available story files with metadata
pub fn list_story_files() -> Result<Vec<(PathBuf, BriefingData)>> {
    let stories_dir = get_default_stories_dir()?;

    let mut files = Vec::new();

    if stories_dir.exists() {
        for entry in fs::read_dir(&stories_dir).context("Failed to read stories directory")? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                match load_stories(&path) {
                    Ok(data) => {
                        files.push((path, data));
                    }
                    Err(e) => {
                        eprintln!("Warning: Could not load {}: {}", path.display(), e);
                    }
                }
            }
        }
    }

    // Sort by creation date (newest first)
    files.sort_by(|a, b| {
        let time_a = DateTime::parse_from_rfc3339(&a.1.created_at).ok();
        let time_b = DateTime::parse_from_rfc3339(&b.1.created_at).ok();
        time_b.cmp(&time_a)
    });

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clustering::{Story, Topic};
    use crate::models::ShowInfo;
    use crate::summarizer::Summary;
    use tempfile::tempdir;

    fn make_test_data() -> BriefingData {
        let show = ShowInfo::new("Test Show", "test", "TEST");
        let story = Story {
            title: "Test Article".to_string(),
            url: "https://example.com".to_string(),
            created: "2026-02-01".to_string(),
            summary: Summary::Editorial {
                whats_happening: "New development announced".to_string(),
                why_it_matters: "It changes the industry".to_string(),
                big_picture: String::new(),
                quote: None,
            },
        };
        let topics = vec![Topic {
            title: "News".to_string(),
            stories: vec![story],
        }];
        BriefingData {
            version: "1.0".to_string(),
            created_at: "2026-02-01T00:00:00Z".to_string(),
            show,
            topics,
        }
    }

    #[test]
    fn test_save_and_load_stories() {
        let temp_dir = tempdir().unwrap();
        let filepath = temp_dir.path().join("test-stories.json");

        let data = make_test_data();
        let json = serde_json::to_string_pretty(&data).unwrap();
        fs::write(&filepath, json).unwrap();

        let loaded = load_stories(&filepath).unwrap();

        assert_eq!(loaded.version, "1.0");
        assert_eq!(loaded.show.name, "Test Show");
        assert_eq!(loaded.topics.len(), 1);
        assert_eq!(loaded.topics[0].stories.len(), 1);
        assert_eq!(loaded.topics[0].stories[0].title, "Test Article");
    }

    #[test]
    fn test_load_stories_file_not_found() {
        let result = load_stories(&PathBuf::from("/nonexistent/path/stories.json"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_load_stories_invalid_json() {
        let temp_dir = tempdir().unwrap();
        let filepath = temp_dir.path().join("invalid.json");
        fs::write(&filepath, "not valid json").unwrap();

        let result = load_stories(&filepath);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_stories_wrong_version() {
        let temp_dir = tempdir().unwrap();
        let filepath = temp_dir.path().join("wrong-version.json");

        let mut data = make_test_data();
        data.version = "2.0".to_string();
        let json = serde_json::to_string_pretty(&data).unwrap();
        fs::write(&filepath, json).unwrap();

        let result = load_stories(&filepath);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported"));
    }

    #[test]
    fn test_load_stories_empty_topics() {
        let temp_dir = tempdir().unwrap();
        let filepath = temp_dir.path().join("empty-topics.json");

        let show = ShowInfo::new("Test", "test", "TEST");
        let data = BriefingData {
            version: "1.0".to_string(),
            created_at: "2026-02-01T00:00:00Z".to_string(),
            show,
            topics: vec![],
        };
        let json = serde_json::to_string_pretty(&data).unwrap();
        fs::write(&filepath, json).unwrap();

        let result = load_stories(&filepath);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no topics"));
    }

    #[test]
    fn test_get_default_stories_dir() {
        let dir = get_default_stories_dir().unwrap();
        assert!(dir.to_string_lossy().contains("podcast-briefing"));
        assert!(dir.to_string_lossy().contains("stories"));
    }
}
