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

fn main() -> Result<()> {
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

    // Use local time for file naming (same as collect-stories)
    let local_now = Local::now();
    let now = Utc
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

    println!("\n📝 Generating HTML briefing...");
    let html_content = shared::briefing::BriefingGenerator::generate(&topics, &show_name, now);
    let html_filepath = shared::briefing::BriefingGenerator::save(&html_content, &show_slug, now)
        .context("Failed to save HTML file")?;

    println!("✓ HTML saved to: {}", html_filepath.display());

    println!("\n📊 Generating links CSV...");
    let csv_content = shared::briefing::BriefingGenerator::generate_links_csv(&topics);
    let csv_filepath =
        shared::briefing::BriefingGenerator::save_links_csv(&csv_content, &show_slug, now)
            .context("Failed to save CSV file")?;

    println!("✓ CSV saved to: {}", csv_filepath.display());

    println!("\n✅ Done! Files ready for upload to Google Docs.");

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
    let mut summary_points: Vec<String> = Vec::new();
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
            summary_points.clear();
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
                        // Check if it's a quote line
                        if trimmed.starts_with('"') && !trimmed.starts_with("- ") {
                            quote = Some(trimmed.to_string());
                        } else if let Some(point) = trimmed.strip_prefix("- ") {
                            summary_points.push(point.trim().to_string());
                        }

                        // If we have accumulated points, create the summary
                        if !summary_points.is_empty() {
                            if let Some(ref mut story) = current_story {
                                story.summary = Summary::Success {
                                    points: summary_points.clone(),
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

    Ok((show_name, topics))
}
