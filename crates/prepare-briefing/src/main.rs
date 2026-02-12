use anyhow::{Context, Result};
use chrono::{Datelike, Local, TimeZone, Timelike, Utc};
use clap::Parser;
use shared::{Story, Summary, Topic};
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
fn log_error(message: &str) {
    let log_path = "/tmp/prepare-briefing-errors.log";
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(file, "[{}] {}", timestamp, message);
    }
}

#[derive(Parser)]
#[command(name = "prepare-briefing")]
#[command(about = "Convert edited org-mode briefing to HTML and CSV for Google Docs")]
struct Args {
    /// Path to the org-mode file (if not provided, will list available files)
    #[arg(short, long)]
    file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let org_file = if let Some(path) = args.file {
        path
    } else {
        select_org_file()?
    };

    println!("📖 Reading org file: {}", org_file.display());
    let org_content = fs::read_to_string(&org_file)
        .context(format!("Failed to read org file: {}", org_file.display()))?;

    println!("🔍 Parsing org-mode content...");
    let (show_name, topics) = parse_org_mode(&org_content)?;

    println!(
        "✓ Parsed {} topics with {} total stories",
        topics.len(),
        topics.iter().map(|t| t.stories.len()).sum::<usize>()
    );

    // Use local time for show date calculation (same as collect-stories)
    let local_now = Local::now();
    let local_as_utc = Utc
        .with_ymd_and_hms(
            local_now.year(),
            local_now.month(),
            local_now.day(),
            local_now.hour(),
            local_now.minute(),
            local_now.second(),
        )
        .unwrap();
    let show_slug = extract_show_slug(&org_file)?;

    // Calculate the show date for the filename (e.g., next Tuesday for MBW)
    let show_date =
        shared::briefing::BriefingGenerator::next_show_datetime(&show_name, local_as_utc);

    println!("\n📝 Generating HTML briefing...");
    let html_content =
        shared::briefing::BriefingGenerator::generate(&topics, &show_name, show_date);
    let html_filepath =
        shared::briefing::BriefingGenerator::save(&html_content, &show_slug, show_date)
            .context("Failed to save HTML file")?;

    println!("✓ HTML saved to: {}", html_filepath.display());

    println!("\n📊 Generating links CSV...");
    let csv_content = shared::briefing::BriefingGenerator::generate_links_csv(&topics);
    let csv_filepath =
        shared::briefing::BriefingGenerator::save_links_csv(&csv_content, &show_slug, show_date)
            .context("Failed to save CSV file")?;

    println!("✓ CSV saved to: {}", csv_filepath.display());

    // Upload to Fastmail WebDAV
    println!("\n☁️  Uploading to Fastmail...");
    match upload_to_fastmail(&show_slug, &html_filepath, &csv_filepath).await {
        Ok(()) => {
            println!("✓ Uploaded to Fastmail WebDAV");
        }
        Err(e) => {
            println!("⚠ Upload failed: {} (files saved locally)", e);
        }
    }

    println!("\n✅ Done!");

    Ok(())
}

async fn upload_to_fastmail(
    show_slug: &str,
    html_path: &Path,
    csv_path: &Path,
) -> Result<()> {
    // Load credentials from .env file
    let env_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
        .join(".config/podcast-briefing/.env");

    dotenvy::from_path(&env_path)
        .context(format!("Failed to load credentials from {}", env_path.display()))?;

    let fastmail_user = std::env::var("FASTMAIL_USER")
        .context("FASTMAIL_USER not set in .env")?;
    let fastmail_password = std::env::var("FASTMAIL_PASSWORD")
        .context("FASTMAIL_PASSWORD not set in .env")?;

    let base_url = "https://myfiles.fastmail.com/Briefings";
    let client = reqwest::Client::new();

    // Upload HTML as index.html
    let html_url = format!("{}/{}/index.html", base_url, show_slug);
    let html_content = fs::read(html_path)
        .context("Failed to read HTML file for upload")?;

    let response = client
        .put(&html_url)
        .basic_auth(&fastmail_user, Some(&fastmail_password))
        .body(html_content)
        .send()
        .await
        .context("Failed to upload HTML")?;

    if !response.status().is_success() {
        anyhow::bail!("HTML upload failed: HTTP {}", response.status());
    }
    println!("  ✓ HTML → {}", html_url);

    // Upload CSV as links.csv
    let csv_url = format!("{}/{}/links.csv", base_url, show_slug);
    let csv_content = fs::read(csv_path)
        .context("Failed to read CSV file for upload")?;

    let response = client
        .put(&csv_url)
        .basic_auth(&fastmail_user, Some(&fastmail_password))
        .body(csv_content)
        .send()
        .await
        .context("Failed to upload CSV")?;

    if !response.status().is_success() {
        anyhow::bail!("CSV upload failed: HTTP {}", response.status());
    }
    println!("  ✓ CSV  → {}", csv_url);

    Ok(())
}

fn select_org_file() -> Result<PathBuf> {
    let documents_dir = dirs::document_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find Documents directory"))?;

    // Find all .org files in Documents directory
    let mut org_files: Vec<PathBuf> = fs::read_dir(&documents_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "org")
                .unwrap_or(false)
        })
        .collect();

    if org_files.is_empty() {
        anyhow::bail!("No .org files found in {}", documents_dir.display());
    }

    // Sort by modification time (newest first)
    org_files.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .map(std::cmp::Reverse)
    });

    println!("Available org files:\n");
    for (i, file) in org_files.iter().enumerate() {
        let filename = file.file_name().unwrap().to_string_lossy();
        let modified = fs::metadata(file)
            .and_then(|m| m.modified())
            .ok()
            .map(|t| {
                let datetime: chrono::DateTime<chrono::Local> = t.into();
                datetime.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_else(|| "unknown".to_string());

        println!("  {}) {} (modified: {})", i + 1, filename, modified);
    }

    print!("\nSelect file (1-{}): ", org_files.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let selection: usize = input
        .trim()
        .parse()
        .context("Invalid selection. Please enter a number.")?;

    if selection < 1 || selection > org_files.len() {
        anyhow::bail!(
            "Selection out of range. Please choose 1-{}",
            org_files.len()
        );
    }

    Ok(org_files[selection - 1].clone())
}

fn extract_show_slug(org_file: &Path) -> Result<String> {
    let filename = org_file
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid filename"))?;

    // Filename format is: {show_slug}-{YYYY-MM-DD}.org
    // Extract just the show slug (everything before the date pattern)
    // Look for pattern: -{YYYY}-{MM}-{DD}
    let parts: Vec<&str> = filename.split('-').collect();

    if parts.len() >= 4 {
        // If we have at least 4 parts, assume last 3 are YYYY-MM-DD
        // Take everything except the last 3 parts
        Ok(parts[..parts.len() - 3].join("-"))
    } else {
        // Fallback: use the first part
        Ok(parts[0].to_string())
    }
}

fn parse_org_mode(content: &str) -> Result<(String, Vec<Topic>)> {
    let lines = content.lines();
    let mut show_name = String::from("Briefing");
    let mut topics: Vec<Topic> = Vec::new();
    let mut current_topic: Option<Topic> = None;
    let mut current_story: Option<Story> = None;
    let mut current_section: Option<String> = None;
    let mut whats_happening: Option<String> = None;
    let mut why_it_matters: Option<String> = None;
    let mut big_picture: Option<String> = None;
    let mut the_product: Option<String> = None;
    let mut cost: Option<String> = None;
    let mut availability: Option<String> = None;
    let mut platforms: Option<String> = None;
    let mut quote: Option<String> = None;

    for line in lines {
        let trimmed = line.trim();

        // Parse title
        if trimmed.starts_with("#+TITLE:") {
            if let Some(title) = trimmed.strip_prefix("#+TITLE:") {
                let title = title.trim();
                // Extract show name from "TWiT Briefing Book" -> "TWiT"
                show_name = title
                    .replace("Briefing Book", "")
                    .replace("Briefing", "")
                    .trim()
                    .to_string();
            }
            continue;
        }

        // Skip other properties
        if trimmed.starts_with("#+") {
            continue;
        }

        // Level 1 heading: Topic
        if let Some(title) = trimmed.strip_prefix("* ") {
            // Save previous topic if exists
            if let Some(mut topic) = current_topic.take() {
                if let Some(story) = current_story.take() {
                    topic.stories.push(story);
                }
                // Only add topics with stories (skip "Back of the Book", etc.)
                if !topic.stories.is_empty() {
                    topics.push(topic);
                }
            }

            // Start new topic
            current_topic = Some(Topic {
                title: title.trim().to_string(),
                stories: Vec::new(),
            });
            current_story = None;
            current_section = None;
            continue;
        }

        // Level 2 heading: Story title
        if let Some(title) = trimmed.strip_prefix("** ") {
            // Save previous story if exists
            if let Some(story) = current_story.take() {
                if let Some(ref mut topic) = current_topic {
                    topic.stories.push(story);
                }
            }

            // Start new story
            current_story = Some(Story {
                title: title.trim().to_string(),
                url: String::new(),
                created: String::new(),
                summary: Summary::Insufficient,
            });
            current_section = None;
            whats_happening = None;
            why_it_matters = None;
            big_picture = None;
            the_product = None;
            cost = None;
            availability = None;
            platforms = None;
            quote = None;
            continue;
        }

        // Level 3 heading: Section (URL or Summary)
        if let Some(section) = trimmed.strip_prefix("*** ") {
            current_section = Some(section.trim().to_string());
            continue;
        }

        // Content lines
        if !trimmed.is_empty() {
            if let Some(ref section) = current_section {
                match section.as_str() {
                    "URL" => {
                        if let Some(ref mut story) = current_story {
                            story.url = trimmed.to_string();
                        }
                    }
                    "Date" => {
                        if let Some(ref mut story) = current_story {
                            story.created = trimmed.to_string();
                        }
                    }
                    "Summary" => {
                        if trimmed.starts_with('"') {
                            quote = Some(trimmed.to_string());
                        } else if let Some(val) = trimmed.strip_prefix("What's happening: ") {
                            whats_happening = Some(val.to_string());
                        } else if let Some(val) = trimmed.strip_prefix("Why it matters: ") {
                            why_it_matters = Some(val.to_string());
                        } else if let Some(val) = trimmed.strip_prefix("The big picture: ") {
                            big_picture = Some(val.to_string());
                        } else if let Some(val) = trimmed.strip_prefix("The product: ") {
                            the_product = Some(val.to_string());
                        } else if let Some(val) = trimmed.strip_prefix("Cost: ") {
                            cost = Some(val.to_string());
                        } else if let Some(val) = trimmed.strip_prefix("Availability: ") {
                            availability = Some(val.to_string());
                        } else if let Some(val) = trimmed.strip_prefix("Platforms: ") {
                            platforms = Some(val.to_string());
                        }

                        // Build summary from accumulated fields
                        if let Some(ref mut story) = current_story {
                            if let Some(ref prod) = the_product {
                                story.summary = Summary::Product {
                                    the_product: prod.clone(),
                                    cost: cost.clone().unwrap_or_default(),
                                    availability: availability.clone().unwrap_or_default(),
                                    platforms: platforms.clone().unwrap_or_default(),
                                    quote: quote.clone(),
                                };
                            } else if let (Some(ref wh), Some(ref wm)) =
                                (&whats_happening, &why_it_matters)
                            {
                                story.summary = Summary::Editorial {
                                    whats_happening: wh.clone(),
                                    why_it_matters: wm.clone(),
                                    big_picture: big_picture.clone().unwrap_or_default(),
                                    quote: quote.clone(),
                                };
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Save last story and topic
    if let Some(story) = current_story {
        if let Some(ref mut topic) = current_topic {
            topic.stories.push(story);
        }
    }
    if let Some(topic) = current_topic {
        if !topic.stories.is_empty() {
            topics.push(topic);
        }
    }

    if topics.is_empty() {
        anyhow::bail!(
            "No topics found in org file. Make sure the file follows the expected format."
        );
    }

    // Preserve org file order — user may have manually reordered stories after editing

    Ok((show_name, topics))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== extract_show_slug Tests ====================

    #[test]
    fn test_extract_show_slug_twit() {
        let path = PathBuf::from("/home/user/Documents/twit-2026-02-01.org");
        let result = extract_show_slug(&path).unwrap();
        assert_eq!(result, "twit");
    }

    #[test]
    fn test_extract_show_slug_mbw() {
        let path = PathBuf::from("/home/user/Documents/mbw-2026-02-03.org");
        let result = extract_show_slug(&path).unwrap();
        assert_eq!(result, "mbw");
    }

    #[test]
    fn test_extract_show_slug_with_hyphens() {
        let path = PathBuf::from("/home/user/Documents/intelligent-machines-2026-02-04.org");
        let result = extract_show_slug(&path).unwrap();
        assert_eq!(result, "intelligent-machines");
    }

    #[test]
    fn test_extract_show_slug_short_name() {
        let path = PathBuf::from("im-2026-02-04.org");
        let result = extract_show_slug(&path).unwrap();
        assert_eq!(result, "im");
    }

    // ==================== parse_org_mode Tests ====================

    #[test]
    fn test_parse_org_mode_editorial() {
        let content = r#"#+TITLE: TWiT Briefing Book
#+DATE: Sun, 2 February 2026

* Apple

** iPhone 17 Announced

*** URL
https://example.com/iphone17

*** Date
2026-02-01

*** Summary
What's happening: Apple announced the iPhone 17 with a new A19 chip.
Why it matters: The new chip delivers 40% better performance.
The big picture: Apple continues to push custom silicon across its lineup.
"#;

        let (show_name, topics) = parse_org_mode(content).unwrap();

        assert_eq!(show_name, "TWiT");
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].title, "Apple");
        assert_eq!(topics[0].stories.len(), 1);
        assert_eq!(topics[0].stories[0].title, "iPhone 17 Announced");
        assert_eq!(topics[0].stories[0].url, "https://example.com/iphone17");

        if let Summary::Editorial {
            whats_happening,
            why_it_matters,
            big_picture,
            ..
        } = &topics[0].stories[0].summary
        {
            assert!(whats_happening.contains("iPhone 17"));
            assert!(why_it_matters.contains("40%"));
            assert!(big_picture.contains("custom silicon"));
        } else {
            panic!("Expected Summary::Editorial");
        }
    }

    #[test]
    fn test_parse_org_mode_with_quote() {
        let content = r#"#+TITLE: Test Briefing

* News

** Story Title

*** URL
https://test.com

*** Summary
"This is a quote" -- Author Name

What's happening: Something happened.
Why it matters: It matters because of reasons.
"#;

        let (_, topics) = parse_org_mode(content).unwrap();

        if let Summary::Editorial { quote, .. } = &topics[0].stories[0].summary {
            assert!(quote.is_some());
            assert!(quote.as_ref().unwrap().contains("This is a quote"));
        } else {
            panic!("Expected Summary::Editorial");
        }
    }

    #[test]
    fn test_parse_org_mode_product() {
        let content = r#"#+TITLE: Test Briefing

* Products

** New Gadget Review

*** URL
https://test.com/gadget

*** Summary
The product: A revolutionary new widget that does everything.
Cost: Starting at $299.
Availability: Ships March 2026.
Platforms: iOS, Android, Web.
"#;

        let (_, topics) = parse_org_mode(content).unwrap();

        if let Summary::Product {
            the_product,
            cost,
            availability,
            platforms,
            ..
        } = &topics[0].stories[0].summary
        {
            assert!(the_product.contains("revolutionary"));
            assert!(cost.contains("$299"));
            assert!(availability.contains("March"));
            assert!(platforms.contains("iOS"));
        } else {
            panic!("Expected Summary::Product");
        }
    }

    #[test]
    fn test_parse_org_mode_multiple_topics() {
        let content = r#"#+TITLE: TWiT Briefing

* Apple

** Apple Story

*** URL
https://apple.com

*** Summary
What's happening: Apple did something.
Why it matters: It matters.

* Google

** Google Story

*** URL
https://google.com

*** Summary
What's happening: Google did something.
Why it matters: It also matters.
"#;

        let (_, topics) = parse_org_mode(content).unwrap();

        assert_eq!(topics.len(), 2);
        assert_eq!(topics[0].title, "Apple");
        assert_eq!(topics[1].title, "Google");
    }

    #[test]
    fn test_parse_org_mode_skips_empty_topics() {
        let content = r#"#+TITLE: Test

* Has Stories

** A Story

*** URL
https://example.com

*** Summary
What's happening: Something happened.
Why it matters: It matters.

* Empty Topic

* In Other News

* Leo's Picks
"#;

        let (_, topics) = parse_org_mode(content).unwrap();

        // Only "Has Stories" should be included
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].title, "Has Stories");
    }

    #[test]
    fn test_parse_org_mode_extracts_show_name() {
        let content = r#"#+TITLE: MacBreak Weekly Briefing Book

* Topic

** Story

*** URL
https://test.com

*** Summary
What's happening: Something happened.
Why it matters: It matters.
"#;

        let (show_name, _) = parse_org_mode(content).unwrap();
        assert_eq!(show_name, "MacBreak Weekly");
    }

    #[test]
    fn test_parse_org_mode_no_topics_error() {
        let content = r#"#+TITLE: Empty Briefing

* In Other News

* Leo's Picks
"#;

        let result = parse_org_mode(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No topics found"));
    }

    #[test]
    fn test_parse_org_mode_with_date() {
        let content = r#"#+TITLE: Test Briefing

* Topic

** Story

*** URL
https://test.com

*** Date
Sat, 1 Feb 2026

*** Summary
What's happening: Something happened.
Why it matters: It matters.
"#;

        let (_, topics) = parse_org_mode(content).unwrap();
        assert_eq!(topics[0].stories[0].created, "Sat, 1 Feb 2026");
    }
}
