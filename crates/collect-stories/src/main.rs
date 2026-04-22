use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Timelike, Utc, Weekday};
use clap::Parser;
use shared::{
    local_wallclock_as_utc, raindrop::Bookmark, ArticleContent, ClaudeSummarizer, Config,
    ContentExtractor, ExtractionResult, RaindropClient, ShowInfo, Story, Summary, TopicClusterer,
};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self as stdio, Write};
use std::path::PathBuf;

fn cache_path() -> PathBuf {
    let dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("podcast-briefing");
    std::fs::create_dir_all(&dir).ok();
    dir.join("summaries.json")
}

fn load_summary_cache() -> HashMap<String, Summary> {
    let path = cache_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn save_summary_cache(cache: &HashMap<String, Summary>) {
    let path = cache_path();
    if let Ok(data) = serde_json::to_string(cache) {
        std::fs::write(&path, data).ok();
    }
}

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

    /// Calculate when the most recent past episode ended.
    /// Takes local wall-clock time as UTC (from `local_wallclock_as_utc`) and
    /// returns a "fake UTC" datetime whose components match local (Pacific)
    /// time at the show's end hour.
    fn previous_show_end(&self, local_now: DateTime<Utc>) -> DateTime<Utc> {
        let (target_weekday, end_hour) = match self {
            Show::TWiT => (Weekday::Sun, 17),               // Sunday 5pm Pacific
            Show::MacBreakWeekly => (Weekday::Tue, 14),      // Tuesday 2pm Pacific
            Show::IntelligentMachines => (Weekday::Wed, 17), // Wednesday 5pm Pacific
        };

        let current_day = local_now.weekday().num_days_from_monday();
        let target_day = target_weekday.num_days_from_monday();

        let days_back = if current_day == target_day {
            if local_now.hour() >= end_hour {
                0 // Show ended today
            } else {
                7 // Before show end, go back to previous week
            }
        } else if current_day > target_day {
            current_day - target_day
        } else {
            7 - (target_day - current_day)
        };

        let target_date = (local_now - Duration::days(days_back as i64)).date_naive();
        target_date
            .and_hms_opt(end_hour, 0, 0)
            .expect("valid end-of-show time")
            .and_utc()
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
    let previous_end = show.previous_show_end(local_as_utc);
    // Raindrop's `created:>` filter is exclusive and date-only. Pass end_date - 1
    // day so bookmarks from the show's end date are returned; we filter client-
    // side below for precise cutoff at the actual end time.
    let since = previous_end - Duration::days(1);

    // Real-UTC equivalent of the local wall-clock end time, for comparing
    // against bookmark.created (which Raindrop returns as UTC).
    let previous_end_utc = Local
        .from_local_datetime(&previous_end.naive_utc())
        .earliest()
        .context("Failed to resolve previous show end in local time")?
        .with_timezone(&Utc);

    println!(
        "  Collecting stories since previous {} ended ({} {})",
        show_info.name,
        previous_end.format("%A, %-d %B"),
        previous_end.format("%-l%P")
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
            previous_end.format("%A, %-d %B %Y")
        );
        return Ok(());
    }

    // Drop bookmarks created before the previous show actually ended
    // (Raindrop's date filter is imprecise, so some boundary-day bookmarks
    // from before the cutoff hour may be included).
    let before_filter = bookmarks.len();
    let bookmarks: Vec<_> = bookmarks
        .into_iter()
        .filter(|b| {
            DateTime::parse_from_rfc3339(&b.created)
                .map(|dt| dt.with_timezone(&Utc) > previous_end_utc)
                .unwrap_or(true)
        })
        .collect();
    let pre_cutoff_removed = before_filter - bookmarks.len();
    if pre_cutoff_removed > 0 {
        println!(
            "🧹 Dropped {} bookmark(s) from before previous show end",
            pre_cutoff_removed
        );
    }

    if bookmarks.is_empty() {
        println!("No bookmarks remain after applying precise cutoff.");
        return Ok(());
    }

    // Deduplicate by URL before expensive extraction/summarization
    let original_count = bookmarks.len();
    let bookmarks = deduplicate_bookmarks(bookmarks);
    let duplicates_removed = original_count - bookmarks.len();
    if duplicates_removed > 0 {
        println!("🗑️  Removed {} duplicate URL(s)", duplicates_removed);
    }

    if bookmarks.is_empty() {
        println!("No unique bookmarks after deduplication.");
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
        // Load cached summaries to avoid re-summarizing
        let mut cache = load_summary_cache();
        let mut cached_count = 0;

        let articles_for_summary: Vec<(String, String)> = content_map
            .iter()
            .filter_map(|(url, content)| {
                if let Some(summary) = cache.get(url) {
                    // Only reuse successful summaries from cache
                    if matches!(summary, Summary::Editorial { .. } | Summary::Product { .. }) {
                        summary_map.insert(url.clone(), summary.clone());
                        cached_count += 1;
                        return None;
                    }
                }
                Some((url.clone(), content.text.clone()))
            })
            .collect();

        let new_count = articles_for_summary.len();
        println!(
            "\n🤖 Summarizing articles with Claude AI... ({} cached, {} new)",
            cached_count, new_count
        );

        if !articles_for_summary.is_empty() {
            let summarizer = ClaudeSummarizer::new()?;

            let summary_results = summarizer
                .summarize_articles_parallel(articles_for_summary)
                .await?;

            for (url, summary) in summary_results {
                // Cache successful summaries for future runs
                if matches!(summary, Summary::Editorial { .. } | Summary::Product { .. }) {
                    cache.insert(url.clone(), summary.clone());
                }
                summary_map.insert(url, summary);
            }

            save_summary_cache(&cache);
        }

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
    let clusterer = TopicClusterer::new().context("Failed to initialize topic clusterer")?;
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

/// Remove bookmarks with duplicate URLs, keeping the most recently created one.
fn deduplicate_bookmarks(bookmarks: Vec<Bookmark>) -> Vec<Bookmark> {
    use std::collections::hash_map::Entry;
    let mut seen: HashMap<String, Bookmark> = HashMap::new();

    for bookmark in bookmarks {
        match seen.entry(bookmark.link.clone()) {
            Entry::Occupied(mut e) => {
                if bookmark.created > e.get().created {
                    e.insert(bookmark);
                }
            }
            Entry::Vacant(e) => {
                e.insert(bookmark);
            }
        }
    }

    seen.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bookmark(id: i64, url: &str, created: &str) -> Bookmark {
        Bookmark {
            id,
            title: format!("Article {}", id),
            link: url.to_string(),
            excerpt: None,
            note: None,
            tags: vec![],
            created: created.to_string(),
        }
    }

    // ==================== Show::from_slug ====================

    #[test]
    fn test_show_from_slug_twit() {
        assert!(Show::from_slug("twit").is_some());
    }

    #[test]
    fn test_show_from_slug_mbw() {
        assert!(Show::from_slug("mbw").is_some());
    }

    #[test]
    fn test_show_from_slug_im() {
        assert!(Show::from_slug("im").is_some());
    }

    #[test]
    fn test_show_from_slug_invalid() {
        assert!(Show::from_slug("invalid").is_none());
    }

    #[test]
    fn test_show_from_slug_case_sensitive() {
        assert!(Show::from_slug("TWiT").is_none());
        assert!(Show::from_slug("MBW").is_none());
    }

    // ==================== Show::info ====================

    #[test]
    fn test_show_info_twit() {
        let info = Show::TWiT.info();
        assert_eq!(info.slug, "twit");
        assert!(info.name.contains("Week in Tech"));
        assert_eq!(info.tag, "#twit");
    }

    #[test]
    fn test_show_info_mbw() {
        let info = Show::MacBreakWeekly.info();
        assert_eq!(info.slug, "mbw");
        assert!(info.name.contains("MacBreak"));
        assert_eq!(info.tag, "#mbw");
    }

    #[test]
    fn test_show_info_im() {
        let info = Show::IntelligentMachines.info();
        assert_eq!(info.slug, "im");
        assert!(info.name.contains("Intelligent"));
        assert_eq!(info.tag, "#im");
    }

    // ==================== Show::previous_show_end ====================
    //
    // previous_show_end takes "fake UTC" — a DateTime<Utc> whose weekday/hour
    // values represent local wall-clock time (Pacific) — and returns a fake-
    // UTC datetime anchored at the show's end hour on the target date
    // (Sun 17:00, Tue 14:00, Wed 17:00).

    /// Helper: create a "fake UTC" datetime with the given weekday and hour.
    /// Uses 2026 dates where we know the actual weekdays.
    fn fake_utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            .and_utc()
    }

    #[test]
    fn test_previous_show_end_twit_sunday_after_cutoff() {
        // Sunday 6pm (hour >= 17) → same day, anchored at 5pm
        let show = Show::TWiT;
        let local_now = fake_utc(2026, 3, 22, 18);
        let end = show.previous_show_end(local_now);
        assert_eq!(end, fake_utc(2026, 3, 22, 17));
    }

    #[test]
    fn test_previous_show_end_twit_sunday_before_cutoff() {
        // Sunday 3pm (hour < 17) → previous Sunday 5pm
        let show = Show::TWiT;
        let local_now = fake_utc(2026, 3, 22, 15);
        let end = show.previous_show_end(local_now);
        assert_eq!(end, fake_utc(2026, 3, 15, 17));
    }

    #[test]
    fn test_previous_show_end_twit_monday() {
        // Monday → previous day (Sunday) at 5pm
        let show = Show::TWiT;
        let local_now = fake_utc(2026, 3, 23, 10);
        let end = show.previous_show_end(local_now);
        assert_eq!(end, fake_utc(2026, 3, 22, 17));
    }

    #[test]
    fn test_previous_show_end_twit_saturday() {
        // Saturday → previous Sunday 5pm (6 days back)
        let show = Show::TWiT;
        let local_now = fake_utc(2026, 3, 21, 14);
        let end = show.previous_show_end(local_now);
        assert_eq!(end, fake_utc(2026, 3, 15, 17));
    }

    #[test]
    fn test_previous_show_end_mbw_tuesday_after_cutoff() {
        // Tuesday 3pm (hour >= 14) → same day, anchored at 2pm
        let show = Show::MacBreakWeekly;
        let local_now = fake_utc(2026, 3, 24, 15);
        let end = show.previous_show_end(local_now);
        assert_eq!(end, fake_utc(2026, 3, 24, 14));
    }

    #[test]
    fn test_previous_show_end_mbw_tuesday_before_cutoff() {
        // Tuesday 8am (hour < 14) → previous Tuesday 2pm
        let show = Show::MacBreakWeekly;
        let local_now = fake_utc(2026, 3, 24, 8);
        let end = show.previous_show_end(local_now);
        assert_eq!(end, fake_utc(2026, 3, 17, 14));
    }

    #[test]
    fn test_previous_show_end_im_wednesday_after_cutoff() {
        // Wednesday 5pm (hour >= 17) → same day, anchored at 5pm
        let show = Show::IntelligentMachines;
        let local_now = fake_utc(2026, 3, 25, 17);
        let end = show.previous_show_end(local_now);
        assert_eq!(end, fake_utc(2026, 3, 25, 17));
    }

    #[test]
    fn test_previous_show_end_im_wednesday_before_cutoff() {
        // Wednesday 10am (hour < 17) → previous Wednesday 5pm
        let show = Show::IntelligentMachines;
        let local_now = fake_utc(2026, 3, 25, 10);
        let end = show.previous_show_end(local_now);
        assert_eq!(end, fake_utc(2026, 3, 18, 17));
    }

    // ==================== deduplicate_bookmarks ====================

    #[test]
    fn test_deduplicate_keeps_first_unique_url() {
        let bookmarks = vec![
            make_bookmark(1, "https://example.com/a", "2026-01-01"),
            make_bookmark(2, "https://example.com/b", "2026-01-02"),
        ];

        let result = deduplicate_bookmarks(bookmarks);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_deduplicate_removes_same_url_different_ids() {
        let bookmarks = vec![
            make_bookmark(1, "https://example.com/article", "2026-01-01"),
            make_bookmark(2, "https://example.com/article", "2026-01-02"),
        ];

        let result = deduplicate_bookmarks(bookmarks);
        assert_eq!(result.len(), 1);
        // Keeps the newer one
        assert_eq!(result[0].id, 2);
    }

    #[test]
    fn test_deduplicate_keeps_newest_created() {
        let bookmarks = vec![
            make_bookmark(1, "https://example.com/article", "2026-01-05"),
            make_bookmark(2, "https://example.com/article", "2026-01-01"),
            make_bookmark(3, "https://example.com/article", "2026-01-10"),
        ];

        let result = deduplicate_bookmarks(bookmarks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, 3); // Newest
    }

    #[test]
    fn test_deduplicate_preserves_different_urls() {
        let bookmarks = vec![
            make_bookmark(1, "https://example.com/a", "2026-01-01"),
            make_bookmark(2, "https://example.com/b", "2026-01-01"),
            make_bookmark(3, "https://example.com/c", "2026-01-01"),
        ];

        let result = deduplicate_bookmarks(bookmarks);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_deduplicate_empty_input() {
        let bookmarks: Vec<Bookmark> = vec![];
        let result = deduplicate_bookmarks(bookmarks);
        assert_eq!(result.len(), 0);
    }
}
