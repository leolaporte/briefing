mod briefing;
mod clustering;
mod config;
mod extractor;
mod raindrop;
mod summarizer;

use anyhow::{Context, Result};
use briefing::BriefingGenerator;
use chrono::{Duration, Utc};
use clustering::{Story, TopicClusterer};
use config::Config;
use extractor::{ArticleContent, ContentExtractor};
use raindrop::RaindropClient;
use std::collections::HashMap;
use std::io::{self, Write};
use summarizer::{ClaudeSummarizer, Summary};

#[derive(Debug, Clone, Copy)]
enum Show {
    TWiT,
    MacBreakWeekly,
    IntelligentMachines,
}

impl Show {
    fn tag(&self) -> &'static str {
        match self {
            Show::TWiT => "#twit",
            Show::MacBreakWeekly => "#mbw",
            Show::IntelligentMachines => "#im",
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Show::TWiT => "TWiT",
            Show::MacBreakWeekly => "MacBreak Weekly",
            Show::IntelligentMachines => "Intelligent Machines",
        }
    }

    fn slug(&self) -> &'static str {
        match self {
            Show::TWiT => "twit",
            Show::MacBreakWeekly => "mbw",
            Show::IntelligentMachines => "im",
        }
    }
}

fn prompt_show_selection() -> Result<Show> {
    println!("Which show?");
    println!("  1) TWiT");
    println!("  2) MacBreak Weekly");
    println!("  3) Intelligent Machines");
    print!("\nEnter your choice (1-3): ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    match input.trim() {
        "1" => Ok(Show::TWiT),
        "2" => Ok(Show::MacBreakWeekly),
        "3" => Ok(Show::IntelligentMachines),
        _ => anyhow::bail!("Invalid selection. Please choose 1, 2, or 3."),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;
    let show = prompt_show_selection()?;

    println!("\n✓ Selected: {}", show.name());

    let now = Utc::now();
    let since = now - Duration::days(7);

    println!("\n📚 Fetching bookmarks from Raindrop.io...");
    let raindrop_client = RaindropClient::new(config.raindrop_api_token)?;
    let bookmarks = raindrop_client
        .fetch_bookmarks(show.tag(), since)
        .await
        .context("Failed to fetch bookmarks")?;

    if bookmarks.is_empty() {
        println!("No bookmarks found with tag {} in the past 7 days.", show.tag());
        return Ok(());
    }

    println!("✓ Found {} bookmarks", bookmarks.len());

    println!("\n🌐 Extracting article content...");
    let extractor = ContentExtractor::new()?;
    let urls: Vec<String> = bookmarks.iter().map(|b| b.link.clone()).collect();
    let content_results = extractor.fetch_articles_parallel(urls).await;

    // Create a map of URL -> ArticleContent for correct pairing (buffer_unordered returns in random order)
    let content_map: HashMap<String, ArticleContent> = content_results
        .into_iter()
        .filter_map(|(url, content)| content.map(|c| (url, c)))
        .collect();

    let articles_with_content: Vec<_> = bookmarks
        .iter()
        .filter_map(|bookmark| {
            content_map.get(&bookmark.link).map(|content| (bookmark, content.clone()))
        })
        .collect();

    let failed_count = bookmarks.len() - articles_with_content.len();
    println!(
        "✓ Successfully extracted content from {}/{} articles",
        articles_with_content.len(),
        bookmarks.len()
    );
    if failed_count > 0 {
        println!("  ({} articles could not be extracted)", failed_count);
    }

    if articles_with_content.is_empty() {
        println!("No article content could be extracted.");
        return Ok(());
    }

    println!("\n🤖 Summarizing articles with Claude AI...");
    println!("  (This may take a minute...)");
    let summarizer = ClaudeSummarizer::new(config.anthropic_api_key.clone())?;

    let articles_for_summary: Vec<(String, String)> = articles_with_content
        .iter()
        .map(|(bookmark, content)| (bookmark.link.clone(), content.text.clone()))
        .collect();

    let summary_results = summarizer.summarize_articles_parallel(articles_for_summary).await;

    // Create a map of URL -> Summary for correct pairing
    let summary_map: HashMap<String, Summary> = summary_results
        .into_iter()
        .collect();

    let stories: Vec<Story> = articles_with_content
        .iter()
        .filter_map(|(bookmark, article_content)| {
            summary_map.get(&bookmark.link).map(|summary| {
                // Use extracted publication date if available, otherwise fall back to bookmark creation date
                let created = article_content
                    .published_date
                    .clone()
                    .unwrap_or_else(|| bookmark.created.clone());

                Story {
                    title: bookmark.title.clone(),
                    url: bookmark.link.clone(),
                    created,
                    summary: summary.clone(),
                }
            })
        })
        .collect();

    let successful_summaries = stories
        .iter()
        .filter(|s| matches!(s.summary, Summary::Success { .. }))
        .count();

    println!("✓ Successfully summarized {}/{} articles", successful_summaries, stories.len());

    println!("\n🔗 Clustering stories by topic...");
    let clusterer = TopicClusterer::new(config.anthropic_api_key)?;
    let topics = clusterer
        .cluster_stories(stories)
        .await
        .context("Failed to cluster stories")?;

    println!("✓ Organized into {} topics", topics.len());

    println!("\n📝 Generating briefing document...");
    let briefing_content = BriefingGenerator::generate(&topics, show.name(), now);
    let filepath = BriefingGenerator::save(&briefing_content, show.slug(), now)
        .context("Failed to save briefing")?;

    println!("📊 Generating links spreadsheet...");
    let links_csv = BriefingGenerator::generate_links_csv(&topics);
    let csv_filepath = BriefingGenerator::save_links_csv(&links_csv, show.slug(), now)
        .context("Failed to save links CSV")?;

    println!("\n✅ Briefing saved to: {}", filepath.display());
    println!("✅ Links CSV saved to: {}", csv_filepath.display());
    println!("\nSummary:");
    println!("  • {} bookmarks found", bookmarks.len());
    println!("  • {} articles extracted", articles_with_content.len());
    println!("  • {} articles summarized", successful_summaries);
    println!("  • {} topics identified", topics.len());

    Ok(())
}
