use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Timelike, Utc, Weekday};
use clap::Parser;
use shared::{
    local_wallclock_as_utc, ArticleContent, ClaudeSummarizer, Config, ContentExtractor,
    ExtractionResult, RaindropClient, ShowInfo, Story, Summary, TopicClusterer,
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

    /// Calculate when the most recent past episode started.
    /// Takes local wall-clock time as UTC (from `local_wallclock_as_utc`).
    fn previous_show_start(&self, local_now: DateTime<Utc>) -> DateTime<Utc> {
        let (target_weekday, start_hour) = match self {
            Show::TWiT => (Weekday::Sun, 13),               // Sunday 1pm Pacific
            Show::MacBreakWeekly => (Weekday::Tue, 10),      // Tuesday 10am Pacific
            Show::IntelligentMachines => (Weekday::Wed, 13), // Wednesday 1pm Pacific
        };

        let current_day = local_now.weekday().num_days_from_monday();
        let target_day = target_weekday.num_days_from_monday();

        let days_back = if current_day == target_day {
            if local_now.hour() >= start_hour {
                0 // Show started today
            } else {
                7 // Before show start, go back to previous week
            }
        } else if current_day > target_day {
            current_day - target_day
        } else {
            7 - (target_day - current_day)
        };

        local_now - Duration::days(days_back as i64)
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

    // Use local time for show date calculation (Pacific time zone)
    let local_as_utc = local_wallclock_as_utc().context("Failed to determine local timestamp")?;

    // Automatically determine lookback window based on show schedule
    let previous_start = show.previous_show_start(local_as_utc);
    // Subtract 1 day so Raindrop's "created:>" includes the show date
    let since = previous_start - Duration::days(1);

    println!(
        "  Collecting stories since previous {} ({})",
        show_info.name,
        previous_start.format("%A, %-d %B")
    );

    println!("\n📚 Fetching bookmarks from Raindrop.io...");
    let raindrop_client = RaindropClient::new(config.raindrop_api_token)?;
    let bookmarks = raindrop_client
        .fetch_bookmarks(&show_info.tag, since)
        .await
        .context("Failed to fetch bookmarks")?;

    if bookmarks.is_empty() {
        println!(
            "No bookmarks found with tag {} since {}.",
            show_info.tag,
            previous_start.format("%A, %-d %B %Y")
        );
        return Ok(());
    }

    println!("✓ Found {} bookmarks", bookmarks.len());

    println!("\n🌐 Extracting article content...");
    let extractor = ContentExtractor::new()?;
    let urls: Vec<String> = bookmarks.iter().map(|b| b.link.clone()).collect();
    let content_results = extractor.fetch_articles_parallel(urls).await;

    // Create maps for successful extractions and paywalled URLs
    let mut content_map: HashMap<String, ArticleContent> = HashMap::new();
    let mut paywalled_urls: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (url, result) in content_results {
        match result {
            ExtractionResult::Success(content) => {
                content_map.insert(url, content);
            }
            ExtractionResult::Paywalled => {
                paywalled_urls.insert(url);
            }
            ExtractionResult::Failed(reason) => {
                log_error(&format!("Failed to extract: {} - {}", url, reason));
            }
        }
    }

    let successful_extractions = content_map.len();
    let paywalled_count = paywalled_urls.len();
    let failed_count = bookmarks.len() - successful_extractions - paywalled_count;

    println!(
        "✓ Extracted {}/{} articles ({} paywalled, {} failed)",
        successful_extractions,
        bookmarks.len(),
        paywalled_count,
        failed_count
    );

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
            .filter(|s| matches!(s, Summary::Editorial { .. } | Summary::Product { .. }))
            .count();

        println!(
            "✓ Successfully summarized {}/{} articles",
            successful_summaries,
            summary_map.len()
        );
    }

    // Helper to create fallback summary from Raindrop note or excerpt fields
    let fallback_summary = |bookmark: &shared::raindrop::Bookmark, reason: &str| -> Summary {
        // Try note first, then excerpt
        for text in [&bookmark.note, &bookmark.excerpt].into_iter().flatten() {
            if !text.trim().is_empty() {
                return Summary::Editorial {
                    lede: text.clone(),
                    nutgraf: String::new(),
                    quote: None,
                };
            }
        }
        Summary::Failed(reason.to_string())
    };

    // Create stories for ALL bookmarks
    let stories: Vec<Story> = bookmarks
        .iter()
        .map(|bookmark| {
            // Check if article was paywalled
            if paywalled_urls.contains(&bookmark.link) {
                return Story {
                    title: bookmark.title.clone(),
                    url: bookmark.link.clone(),
                    created: bookmark.created.clone(),
                    summary: fallback_summary(bookmark, "Paywalled - summary unavailable"),
                };
            }

            // Check if we have content
            if let Some(article_content) = content_map.get(&bookmark.link) {
                let created = article_content
                    .published_date
                    .clone()
                    .unwrap_or_else(|| bookmark.created.clone());

                let summary = summary_map
                    .get(&bookmark.link)
                    .cloned()
                    .unwrap_or_else(|| fallback_summary(bookmark, "Summarization failed"));

                return Story {
                    title: bookmark.title.clone(),
                    url: bookmark.link.clone(),
                    created,
                    summary,
                };
            }

            // No content extracted - use excerpt if available
            Story {
                title: bookmark.title.clone(),
                url: bookmark.link.clone(),
                created: bookmark.created.clone(),
                summary: fallback_summary(bookmark, "Summary not available"),
            }
        })
        .collect();

    println!(
        "\n📊 Total stories: {} ({}  successfully summarized, {} failed)",
        stories.len(),
        stories
            .iter()
            .filter(|s| matches!(
                s.summary,
                Summary::Editorial { .. } | Summary::Product { .. }
            ))
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
    // Calculate the show date for the filename (e.g., next Tuesday for MBW)
    let show_date =
        shared::briefing::BriefingGenerator::next_show_datetime(&show_info.name, local_as_utc);
    let org_content = shared::briefing::BriefingGenerator::generate_org_mode(
        &topics,
        &show_info.name,
        local_as_utc,
    );
    let org_filepath = shared::briefing::BriefingGenerator::save_org_mode(
        &org_content,
        &show_info.slug,
        show_date,
    )
    .context("Failed to save org-mode file")?;

    println!(
        "\n✅ Org-mode document saved to: {}",
        org_filepath.display()
    );

    Ok(())
}
