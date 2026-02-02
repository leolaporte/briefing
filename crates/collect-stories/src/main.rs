use anyhow::{Context, Result};
use chrono::{Duration, Local, Utc};
use clap::Parser;
use shared::{
    ArticleContent, ClaudeSummarizer, Config, ContentExtractor, RaindropClient, ShowInfo, Story,
    Summary, TopicClusterer,
};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self as stdio, Write};

#[derive(Debug, Clone, Copy)]
enum Show {
    TWiT,
    MacBreakWeekly,
    IntelligentMachines,
}

impl Show {
    fn info(&self) -> ShowInfo {
        match self {
            Show::TWiT => ShowInfo::new("This Week in Tech", "twit", "#twit"),
            Show::MacBreakWeekly => ShowInfo::new("MacBreak Weekly", "mbw", "#mbw"),
            Show::IntelligentMachines => ShowInfo::new("Intelligent Machines", "im", "#im"),
        }
    }

    fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "twit" => Some(Show::TWiT),
            "mbw" => Some(Show::MacBreakWeekly),
            "im" => Some(Show::IntelligentMachines),
            _ => None,
        }
    }
}

fn log_error(message: &str) {
    let log_path = "/tmp/collect-stories-errors.log";
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(file, "[{}] {}", timestamp, message);
    }
}

fn prompt_show_selection() -> Result<Show> {
    println!("Which show?");
    println!("  1) twit (This Week in Tech)");
    println!("  2) mbw (MacBreak Weekly)");
    println!("  3) im (Intelligent Machines)");
    print!("\nEnter your choice (1-3): ");
    stdio::stdout().flush()?;

    let mut input = String::new();
    stdio::stdin().read_line(&mut input)?;

    match input.trim() {
        "1" => Ok(Show::TWiT),
        "2" => Ok(Show::MacBreakWeekly),
        "3" => Ok(Show::IntelligentMachines),
        _ => anyhow::bail!("Invalid selection. Please choose 1, 2, or 3."),
    }
}

#[derive(Parser)]
#[command(name = "collect-stories")]
#[command(about = "Collect and summarize stories from Raindrop.io for podcast briefing")]
struct Args {
    /// Show to collect stories for (twit, mbw, im)
    #[arg(short, long)]
    show: Option<String>,

    /// Number of days to look back for bookmarks
    #[arg(short, long, default_value = "7")]
    days: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::from_env()?;

    // Determine which show to use
    let show = if let Some(slug) = args.show {
        Show::from_slug(&slug)
            .ok_or_else(|| anyhow::anyhow!("Invalid show: {}. Use 'twit', 'mbw', or 'im'", slug))?
    } else {
        prompt_show_selection()?
    };

    let show_info = show.info();
    println!("\n✓ Selected: {}", show_info.name);

    let now = Utc::now();
    let since = now - Duration::days(args.days);

    // Use local time for show date calculation (Pacific time zone)
    let local_now = Local::now().with_timezone(&Utc);

    println!("\n📚 Fetching bookmarks from Raindrop.io...");
    let raindrop_client = RaindropClient::new(config.raindrop_api_token)?;
    let bookmarks = raindrop_client
        .fetch_bookmarks(&show_info.tag, since)
        .await
        .context("Failed to fetch bookmarks")?;

    if bookmarks.is_empty() {
        println!(
            "No bookmarks found with tag {} in the past {} days.",
            show_info.tag, args.days
        );
        return Ok(());
    }

    println!("✓ Found {} bookmarks", bookmarks.len());

    println!("\n🌐 Extracting article content...");
    let extractor = ContentExtractor::new()?;
    let urls: Vec<String> = bookmarks.iter().map(|b| b.link.clone()).collect();
    let content_results = extractor.fetch_articles_parallel(urls).await;

    // Create a map of URL -> ArticleContent for correct pairing
    let content_map: HashMap<String, ArticleContent> = content_results
        .into_iter()
        .filter_map(|(url, content)| content.map(|c| (url, c)))
        .collect();

    let successful_extractions = content_map.len();
    let failed_count = bookmarks.len() - successful_extractions;

    println!(
        "✓ Successfully extracted content from {}/{} articles",
        successful_extractions,
        bookmarks.len()
    );

    // Log failed extractions to error log
    if failed_count > 0 {
        for bookmark in &bookmarks {
            if !content_map.contains_key(&bookmark.link) {
                log_error(&format!(
                    "Failed to extract article: \"{}\" - URL: {}",
                    bookmark.title, bookmark.link
                ));
            }
        }
    }

    // Only summarize articles that have content
    let mut summary_map: HashMap<String, Summary> = HashMap::new();

    if !content_map.is_empty() {
        println!("\n🤖 Summarizing articles with Claude AI...");
        println!("  (This may take a minute...)");
        let summarizer = ClaudeSummarizer::new(config.anthropic_api_key.clone())?;

        let articles_for_summary: Vec<(String, String)> = content_map
            .iter()
            .map(|(url, content)| (url.clone(), content.text.clone()))
            .collect();

        let summary_results = summarizer
            .summarize_articles_parallel(articles_for_summary)
            .await;

        summary_map = summary_results.into_iter().collect();

        let successful_summaries = summary_map
            .values()
            .filter(|s| matches!(s, Summary::Success { .. }))
            .count();

        println!(
            "✓ Successfully summarized {}/{} articles",
            successful_summaries,
            summary_map.len()
        );
    }

    // Create stories for ALL bookmarks
    let stories: Vec<Story> = bookmarks
        .iter()
        .map(|bookmark| {
            // Determine the summary to use
            let summary = if let Some(article_content) = content_map.get(&bookmark.link) {
                // We have content - use the summary (or Failed if summarization failed)
                let created = article_content
                    .published_date
                    .clone()
                    .unwrap_or_else(|| bookmark.created.clone());

                let summary = summary_map
                    .get(&bookmark.link)
                    .cloned()
                    .unwrap_or_else(|| Summary::Failed("Summarization failed".to_string()));

                return Story {
                    title: bookmark.title.clone(),
                    url: bookmark.link.clone(),
                    created,
                    summary,
                };
            } else {
                // No content extracted
                Summary::Failed("Summary not available".to_string())
            };

            Story {
                title: bookmark.title.clone(),
                url: bookmark.link.clone(),
                created: bookmark.created.clone(),
                summary,
            }
        })
        .collect();

    println!(
        "\n📊 Total stories: {} ({}  successfully summarized, {} failed)",
        stories.len(),
        stories
            .iter()
            .filter(|s| matches!(s.summary, Summary::Success { .. }))
            .count(),
        stories
            .iter()
            .filter(|s| matches!(s.summary, Summary::Failed(_)))
            .count()
    );

    println!("\n🔗 Clustering stories by topic...");
    let clusterer = TopicClusterer::new(config.anthropic_api_key)?;
    let topics = clusterer
        .cluster_stories(stories)
        .await
        .context("Failed to cluster stories")?;

    println!("✓ Organized into {} topics", topics.len());

    println!("\n📝 Generating org-mode document...");
    let org_content =
        shared::briefing::BriefingGenerator::generate_org_mode(&topics, &show_info.name, local_now);
    let org_filepath =
        shared::briefing::BriefingGenerator::save_org_mode(&org_content, &show_info.slug, local_now)
            .context("Failed to save org-mode file")?;

    println!(
        "\n✅ Org-mode document saved to: {}",
        org_filepath.display()
    );

    Ok(())
}
