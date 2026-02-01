use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use clap::Parser;
use shared::{
    ArticleContent, ClaudeSummarizer, Config, ContentExtractor, RaindropClient, ShowInfo, Story,
    Summary, TopicClusterer,
};
use std::collections::HashMap;
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

fn prompt_show_selection() -> Result<Show> {
    println!("Which show?");
    println!("  1) TWiT");
    println!("  2) MacBreak Weekly");
    println!("  3) Intelligent Machines");
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

    let articles_with_content: Vec<_> = bookmarks
        .iter()
        .filter_map(|bookmark| {
            content_map
                .get(&bookmark.link)
                .map(|content| (bookmark, content.clone()))
        })
        .collect();

    let failed_count = bookmarks.len() - articles_with_content.len();
    println!(
        "✓ Successfully extracted content from {}/{} articles",
        articles_with_content.len(),
        bookmarks.len()
    );
    if failed_count > 0 {
        println!("\n⚠ Failed to extract {} articles:", failed_count);
        for bookmark in &bookmarks {
            if !content_map.contains_key(&bookmark.link) {
                println!("  ✗ \"{}\"", bookmark.title);
                println!("    URL: {}", bookmark.link);
            }
        }
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

    let summary_results = summarizer
        .summarize_articles_parallel(articles_for_summary)
        .await;

    // Create a map of URL -> Summary for correct pairing
    let summary_map: HashMap<String, Summary> = summary_results.into_iter().collect();

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

    println!(
        "✓ Successfully summarized {}/{} articles",
        successful_summaries,
        stories.len()
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
        shared::briefing::BriefingGenerator::generate_org_mode(&topics, &show_info.name, now);
    let org_filepath =
        shared::briefing::BriefingGenerator::save_org_mode(&org_content, &show_info.slug, now)
            .context("Failed to save org-mode file")?;

    println!(
        "\n✅ Org-mode document saved to: {}",
        org_filepath.display()
    );

    Ok(())
}
