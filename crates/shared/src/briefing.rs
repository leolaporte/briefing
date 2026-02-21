use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use std::fs;
use std::path::PathBuf;

use crate::clustering::Topic;
use crate::summarizer::Summary;

pub struct BriefingGenerator;

impl BriefingGenerator {
    fn format_date(date_str: &str) -> String {
        // Try RFC 3339 first (e.g., "2026-02-07T02:15:35.268Z")
        if let Ok(dt) = date_str.parse::<DateTime<Utc>>() {
            return dt.format("%-d-%b-%Y %-I:%M%p").to_string();
        }
        // Try common date-only formats (legacy org files)
        for fmt in &["%a, %e %b %Y", "%a, %d %b %Y", "%Y-%m-%d"] {
            if let Ok(nd) = chrono::NaiveDate::parse_from_str(date_str.trim(), fmt) {
                return nd.format("%-d-%b-%Y").to_string();
            }
        }
        // Fallback to original string
        date_str.to_string()
    }

    /// Calculate the next show date as a DateTime
    pub fn next_show_datetime(show_name: &str, from_date: DateTime<Utc>) -> DateTime<Utc> {
        use chrono::{Datelike, Timelike, Weekday};

        // Show schedule: (target weekday, cutoff hour in Pacific time)
        // After the cutoff hour on show day, we target NEXT week's show
        let (target_weekday, cutoff_hour) = match show_name {
            "This Week in Tech" => (Weekday::Sun, 18),    // 6p Pacific
            "MacBreak Weekly" => (Weekday::Tue, 14),      // 2p Pacific
            "Intelligent Machines" => (Weekday::Wed, 18), // 6p Pacific
            _ => (Weekday::Sun, 18),                      // Default to Sunday 6p
        };

        let current_day = from_date.weekday().num_days_from_monday();
        let target_day = target_weekday.num_days_from_monday();
        let current_hour = from_date.hour();

        // Calculate days until next occurrence of target day
        let days_until_target = if current_day == target_day {
            // Today is show day - check if we're past the cutoff
            if current_hour >= cutoff_hour {
                7 // Past cutoff, use next week
            } else {
                0 // Before cutoff, use today
            }
        } else if current_day < target_day {
            // Target day is later this week
            target_day - current_day
        } else {
            // Target day already passed this week, use next week
            7 - (current_day - target_day)
        };

        from_date + chrono::Duration::days(days_until_target as i64)
    }

    fn calculate_next_show_date(show_name: &str, from_date: DateTime<Utc>) -> String {
        let next_show = Self::next_show_datetime(show_name, from_date);
        // Format as "Tue, 3 February 2026"
        next_show.format("%a, %-d %B %Y").to_string()
    }

    pub fn generate(topics: &[Topic], show_name: &str, date: DateTime<Utc>) -> String {
        let mut html = String::new();

        // Format date as "Sunday, 1 February 2026"
        let formatted_date = date.format("%A, %-d %B %Y").to_string();

        // HTML header with styling
        html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
        html.push_str("  <meta charset=\"UTF-8\">\n");
        html.push_str(&format!(
            "  <title>{} Briefing - {}</title>\n",
            show_name, formatted_date
        ));
        html.push_str("  <style>\n");
        html.push_str("    body { font-family: Arial, sans-serif; max-width: 900px; margin: 40px auto; padding: 0 20px; line-height: 1.6; }\n");
        html.push_str("    h1 { color: #2c3e50; border-bottom: 3px solid #3498db; padding-bottom: 10px; text-align: center; }\n");
        html.push_str(
            "    h1 .show-name { display: block; font-size: 1.2em; margin-bottom: 10px; }\n",
        );
        html.push_str("    h1 .date { display: block; font-size: 0.8em; font-weight: normal; color: #555; }\n");
        html.push_str("    h1 .prepared { display: block; font-size: 0.7em; font-weight: normal; color: #888; margin-top: 5px; }\n");
        html.push_str("    h2 { color: #34495e; margin: 0; padding: 10px; background-color: #ecf0f1; border-left: 4px solid #3498db; }\n");
        html.push_str("    h3 { color: #2c3e50; margin-top: 25px; }\n");
        html.push_str("    .metadata { color: #7f8c8d; font-size: 0.9em; margin: 5px 0; }\n");
        html.push_str("    .link { color: #3498db; text-decoration: none; }\n");
        html.push_str("    .link:hover { text-decoration: underline; }\n");
        html.push_str("    details.topic { margin: 40px 0 20px 0; }\n");
        html.push_str(
            "    details.topic > summary { display: block; cursor: pointer; user-select: none; }\n",
        );
        html.push_str("    details.topic > summary:hover h2 { background-color: #d5dbdb; }\n");
        html.push_str(
            "    details.topic > summary h2::before { content: '▼ '; font-size: 0.8em; }\n",
        );
        html.push_str("    details.topic:not([open]) > summary h2::before { content: '▶ '; }\n");
        html.push_str("    details.article { margin: 15px 0; padding: 10px; background-color: #f8f9fa; border-radius: 4px; }\n");
        html.push_str("    details.article > summary { display: none; }\n");
        html.push_str("    ul { margin: 10px 0; padding-left: 20px; }\n");
        html.push_str("    li { margin: 8px 0; }\n");
        html.push_str("    hr { border: none; border-top: 1px solid #ddd; margin: 30px 0; }\n");
        html.push_str("    .error { color: #e74c3c; font-style: italic; }\n");
        html.push_str("  </style>\n");
        html.push_str("</head>\n<body>\n");

        // Main title (three lines)
        // Get current local time for "Prepared" timestamp
        let prepared_time = Local::now();
        // Determine PST/PDT based on UTC offset (-8 = PST, -7 = PDT)
        let tz_abbrev = if prepared_time.offset().local_minus_utc() == -8 * 3600 {
            "PST"
        } else {
            "PDT"
        };
        let prepared_str = format!(
            "{} {}",
            prepared_time.format("%a %-d %b %Y at %H:%M"),
            tz_abbrev
        );

        html.push_str(&format!(
            "<h1><span class=\"show-name\">{} Briefing</span><span class=\"date\">For {}</span><span class=\"prepared\">(Prepared {})</span></h1>\n",
            show_name, formatted_date, prepared_str
        ));

        // Topics
        for (index, topic) in topics.iter().enumerate() {
            html.push_str("<details class=\"topic\">\n");
            html.push_str(&format!(
                "  <summary><h2>{}. {}</h2></summary>\n",
                index + 1,
                Self::escape_html(&topic.title)
            ));
            html.push_str("  <div>\n");

            for story in &topic.stories {
                html.push_str(&format!(
                    "    <h3>{}</h3>\n",
                    Self::escape_html(&story.title)
                ));
                html.push_str("    <div class=\"metadata\">\n");
                html.push_str(&format!(
                    "      <strong>Link:</strong> <a href=\"{}\" class=\"link\" target=\"_blank\">{}</a><br>\n",
                    story.url,
                    Self::escape_html(&story.url)
                ));
                html.push_str(&format!(
                    "      <strong>Date:</strong> {}\n",
                    Self::format_date(&story.created)
                ));
                html.push_str("    </div>\n");

                match &story.summary {
                    Summary::Editorial {
                        lede,
                        nutgraf,
                        quote,
                    } => {
                        html.push_str("    <details class=\"article\" open>\n");
                        html.push_str("      <summary></summary>\n");
                        if let Some(q) = quote {
                            html.push_str(&format!(
                                "      <p>{}</p>\n",
                                Self::format_quote_html(q)
                            ));
                        }
                        html.push_str(&format!(
                            "      <p><strong>{}</strong></p>\n",
                            Self::escape_html(lede)
                        ));
                        html.push_str(&format!("      <p>{}</p>\n", Self::escape_html(nutgraf)));
                        html.push_str("    </details>\n");
                    }
                    Summary::Product {
                        the_product,
                        cost,
                        availability,
                        platforms,
                        quote,
                    } => {
                        html.push_str("    <details class=\"article\" open>\n");
                        html.push_str("      <summary></summary>\n");
                        if let Some(q) = quote {
                            html.push_str(&format!(
                                "      <p>{}</p>\n",
                                Self::format_quote_html(q)
                            ));
                        }
                        html.push_str(&format!(
                            "      <p><strong>The product:</strong> {}</p>\n",
                            Self::escape_html(the_product)
                        ));
                        if !cost.is_empty() {
                            html.push_str(&format!(
                                "      <p><strong>Cost:</strong> {}</p>\n",
                                Self::escape_html(cost)
                            ));
                        }
                        if !availability.is_empty() {
                            html.push_str(&format!(
                                "      <p><strong>Availability:</strong> {}</p>\n",
                                Self::escape_html(availability)
                            ));
                        }
                        if !platforms.is_empty() {
                            html.push_str(&format!(
                                "      <p><strong>Platforms:</strong> {}</p>\n",
                                Self::escape_html(platforms)
                            ));
                        }
                        html.push_str("    </details>\n");
                    }
                    Summary::Insufficient | Summary::Failed(_) => {
                        html.push_str("    <p class=\"error\">Summary not available</p>\n");
                    }
                }

                html.push_str("    <hr>\n");
            }

            html.push_str("  </div>\n");
            html.push_str("</details>\n");
        }

        // Add footer section
        html.push_str("<hr style=\"margin: 60px 0 30px 0; border-top: 2px solid #3498db;\">\n");
        html.push_str("<h2 style=\"text-align: center; color: #2c3e50;\">Stories will be updated as needed until show time.</h2>\n");

        html.push_str("</body>\n</html>");
        html
    }

    fn escape_html(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }

    /// Format a quote for HTML: italic quote text, normal attribution
    /// Input: `"quote text" -- Speaker Name`
    /// Output: `<em>"quote text"</em> -- Speaker Name`
    fn format_quote_html(quote: &str) -> String {
        if let Some(sep_pos) = quote.find(" -- ") {
            let quote_text = Self::escape_html(&quote[..sep_pos]);
            let attribution = Self::escape_html(&quote[sep_pos..]);
            format!("<em>{}</em>{}", quote_text, attribution)
        } else {
            format!("<em>{}</em>", Self::escape_html(quote))
        }
    }

    pub fn generate_links_csv(topics: &[Topic]) -> String {
        let mut csv = String::new();

        for topic in topics {
            let mut first_article = true;

            for story in &topic.stories {
                if first_article {
                    // First article row: blank A, topic title in B, article title in C, blank D, link in E
                    csv.push_str(&format!(
                        ",{},{},,{}\n",
                        Self::escape_csv(&topic.title),
                        Self::escape_csv(&story.title),
                        Self::escape_csv(&story.url)
                    ));
                    first_article = false;
                } else {
                    // Subsequent article rows: blank A, blank B, title in C, blank D, link in E
                    csv.push_str(&format!(
                        ",,{},,{}\n",
                        Self::escape_csv(&story.title),
                        Self::escape_csv(&story.url)
                    ));
                }
            }

            // Blank row between topics
            csv.push_str(",,,,\n");
        }

        csv
    }

    fn escape_csv(text: &str) -> String {
        // If the text contains comma, quote, or newline, wrap in quotes and escape quotes
        if text.contains(',') || text.contains('"') || text.contains('\n') {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_string()
        }
    }

    pub fn save(content: &str, show_slug: &str, date: DateTime<Utc>) -> Result<PathBuf> {
        let filename = format!("{}-{}.html", show_slug, date.format("%Y-%m-%d"));

        let documents_dir = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        let filepath = documents_dir.join(&filename);

        fs::write(&filepath, content).context("Failed to write briefing file")?;

        Ok(filepath)
    }

    pub fn save_links_csv(content: &str, show_slug: &str, date: DateTime<Utc>) -> Result<PathBuf> {
        let filename = format!("{}-{}-LINKS.csv", show_slug, date.format("%Y-%m-%d"));

        let documents_dir = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        let filepath = documents_dir.join(&filename);

        fs::write(&filepath, content).context("Failed to write links CSV file")?;

        Ok(filepath)
    }

    pub fn generate_org_mode(topics: &[Topic], show_name: &str, date: DateTime<Utc>) -> String {
        let mut org = String::new();

        let next_show_date = Self::calculate_next_show_date(show_name, date);

        // Properties
        org.push_str(&format!("#+TITLE: {} Briefing Book\n", show_name));
        org.push_str(&format!("#+DATE: {}\n\n", next_show_date));

        // Topics
        for topic in topics {
            org.push_str(&format!("* {}\n\n", topic.title));

            for story in &topic.stories {
                // Article title
                org.push_str(&format!("** {}\n\n", story.title));

                // URL
                org.push_str(&format!("*** URL\n{}\n\n", story.url));

                // Date
                if !story.created.is_empty() {
                    org.push_str(&format!("*** Date\n{}\n\n", story.created));
                }

                // Summary
                org.push_str("*** Summary\n");
                match &story.summary {
                    Summary::Editorial {
                        lede,
                        nutgraf,
                        quote,
                    } => {
                        if let Some(q) = quote {
                            org.push_str(&format!("{}\n\n", q));
                        }
                        org.push_str(&format!("LEDE: {}\n\n", lede));
                        org.push_str(&format!("NUTGRAF: {}\n", nutgraf));
                    }
                    Summary::Product {
                        the_product,
                        cost,
                        availability,
                        platforms,
                        quote,
                    } => {
                        if let Some(q) = quote {
                            org.push_str(&format!("{}\n\n", q));
                        }
                        org.push_str(&format!("The product: {}\n", the_product));
                        if !cost.is_empty() {
                            org.push_str(&format!("Cost: {}\n", cost));
                        }
                        if !availability.is_empty() {
                            org.push_str(&format!("Availability: {}\n", availability));
                        }
                        if !platforms.is_empty() {
                            org.push_str(&format!("Platforms: {}\n", platforms));
                        }
                    }
                    Summary::Insufficient | Summary::Failed(_) => {
                        org.push_str("Summary not available\n");
                    }
                }
                org.push('\n');
            }
        }

        // Add three empty topics at the end
        org.push_str("* In Other News\n\n");
        org.push_str("* Leo's Picks\n\n");
        org.push_str("* In Memoriam\n\n");

        org
    }

    pub fn save_org_mode(content: &str, show_slug: &str, date: DateTime<Utc>) -> Result<PathBuf> {
        let filename = format!("{}-{}.org", show_slug, date.format("%Y-%m-%d"));

        let documents_dir = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        let filepath = documents_dir.join(&filename);

        fs::write(&filepath, content).context("Failed to write org-mode file")?;

        Ok(filepath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Story;
    use chrono::TimeZone;

    #[test]
    fn test_mbw_from_sunday_evening() {
        // Sunday Feb 1, 2026 at 9:25 PM -> next MBW is Tuesday Feb 3
        let date = Utc.with_ymd_and_hms(2026, 2, 1, 21, 25, 0).unwrap();
        let result = BriefingGenerator::calculate_next_show_date("MacBreak Weekly", date);
        assert_eq!(result, "Tue, 3 February 2026");
    }

    #[test]
    fn test_twit_from_sunday_after_cutoff() {
        // Sunday Feb 1, 2026 at 7 PM (after 6 PM cutoff) -> next TWiT is Feb 8
        let date = Utc.with_ymd_and_hms(2026, 2, 1, 19, 0, 0).unwrap();
        let result = BriefingGenerator::calculate_next_show_date("This Week in Tech", date);
        assert_eq!(result, "Sun, 8 February 2026");
    }

    #[test]
    fn test_twit_from_sunday_before_cutoff() {
        // Sunday Feb 1, 2026 at 5 PM (before 6 PM cutoff) -> TWiT is today
        let date = Utc.with_ymd_and_hms(2026, 2, 1, 17, 0, 0).unwrap();
        let result = BriefingGenerator::calculate_next_show_date("This Week in Tech", date);
        assert_eq!(result, "Sun, 1 February 2026");
    }

    #[test]
    fn test_mbw_from_tuesday_after_cutoff() {
        // Tuesday Feb 3, 2026 at 3 PM (after 2 PM cutoff) -> next MBW is Feb 10
        let date = Utc.with_ymd_and_hms(2026, 2, 3, 15, 0, 0).unwrap();
        let result = BriefingGenerator::calculate_next_show_date("MacBreak Weekly", date);
        assert_eq!(result, "Tue, 10 February 2026");
    }

    #[test]
    fn test_mbw_from_tuesday_before_cutoff() {
        // Tuesday Feb 3, 2026 at 1 PM (before 2 PM cutoff) -> MBW is today
        let date = Utc.with_ymd_and_hms(2026, 2, 3, 13, 0, 0).unwrap();
        let result = BriefingGenerator::calculate_next_show_date("MacBreak Weekly", date);
        assert_eq!(result, "Tue, 3 February 2026");
    }

    #[test]
    fn test_im_from_wednesday_after_cutoff() {
        // Wednesday Feb 4, 2026 at 7 PM (after 6 PM cutoff) -> next IM is Feb 11
        let date = Utc.with_ymd_and_hms(2026, 2, 4, 19, 0, 0).unwrap();
        let result = BriefingGenerator::calculate_next_show_date("Intelligent Machines", date);
        assert_eq!(result, "Wed, 11 February 2026");
    }

    #[test]
    fn test_im_from_sunday() {
        // Sunday Feb 1, 2026 -> next IM is Wednesday Feb 4
        let date = Utc.with_ymd_and_hms(2026, 2, 1, 21, 25, 0).unwrap();
        let result = BriefingGenerator::calculate_next_show_date("Intelligent Machines", date);
        assert_eq!(result, "Wed, 4 February 2026");
    }

    // ==================== HTML Escaping Tests ====================

    #[test]
    fn test_escape_html_ampersand() {
        assert_eq!(BriefingGenerator::escape_html("A & B"), "A &amp; B");
    }

    #[test]
    fn test_escape_html_less_than() {
        assert_eq!(BriefingGenerator::escape_html("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn test_escape_html_quotes() {
        assert_eq!(
            BriefingGenerator::escape_html("He said \"hello\""),
            "He said &quot;hello&quot;"
        );
    }

    #[test]
    fn test_escape_html_single_quotes() {
        assert_eq!(BriefingGenerator::escape_html("It's here"), "It&#39;s here");
    }

    #[test]
    fn test_escape_html_combined() {
        assert_eq!(
            BriefingGenerator::escape_html("<a href=\"test\">Click & Go</a>"),
            "&lt;a href=&quot;test&quot;&gt;Click &amp; Go&lt;/a&gt;"
        );
    }

    // ==================== CSV Escaping Tests ====================

    #[test]
    fn test_escape_csv_no_special_chars() {
        assert_eq!(BriefingGenerator::escape_csv("Hello World"), "Hello World");
    }

    #[test]
    fn test_escape_csv_with_comma() {
        assert_eq!(
            BriefingGenerator::escape_csv("Hello, World"),
            "\"Hello, World\""
        );
    }

    #[test]
    fn test_escape_csv_with_quotes() {
        assert_eq!(
            BriefingGenerator::escape_csv("He said \"hi\""),
            "\"He said \"\"hi\"\"\""
        );
    }

    #[test]
    fn test_escape_csv_with_newline() {
        assert_eq!(
            BriefingGenerator::escape_csv("Line1\nLine2"),
            "\"Line1\nLine2\""
        );
    }

    // ==================== Date Formatting Tests ====================

    #[test]
    fn test_format_date_valid_iso() {
        let result = BriefingGenerator::format_date("2026-02-01T15:30:00Z");
        assert_eq!(result, "1-Feb-2026 3:30PM");
    }

    #[test]
    fn test_format_date_invalid_fallback() {
        let result = BriefingGenerator::format_date("not a date");
        assert_eq!(result, "not a date");
    }

    // ==================== HTML Generation Tests ====================

    #[test]
    fn test_generate_html_contains_show_name() {
        use crate::summarizer::Summary;

        let topics = vec![Topic {
            title: "Tech News".to_string(),
            stories: vec![Story {
                title: "Test Article".to_string(),
                url: "https://example.com".to_string(),
                created: "2026-02-01T00:00:00Z".to_string(),
                summary: Summary::Editorial {
                    lede: "Acme Corp announced a new widget".to_string(),
                    nutgraf: "The announcement matters because it changes the industry".to_string(),
                    quote: None,
                },
            }],
        }];

        let date = Utc.with_ymd_and_hms(2026, 2, 1, 12, 0, 0).unwrap();
        let html = BriefingGenerator::generate(&topics, "TWiT", date);

        assert!(html.contains("TWiT Briefing"));
        assert!(html.contains("Tech News"));
        assert!(html.contains("Test Article"));
        assert!(html.contains("https://example.com"));
        assert!(html.contains("Acme Corp announced a new widget"));
    }

    #[test]
    fn test_generate_html_escapes_special_chars() {
        use crate::summarizer::Summary;

        let topics = vec![Topic {
            title: "Apple & Google".to_string(),
            stories: vec![Story {
                title: "Test <script>".to_string(),
                url: "https://example.com".to_string(),
                created: "2026-02-01".to_string(),
                summary: Summary::Editorial {
                    lede: "Point \"quoted\"".to_string(),
                    nutgraf: "It matters".to_string(),
                    quote: None,
                },
            }],
        }];

        let date = Utc.with_ymd_and_hms(2026, 2, 1, 12, 0, 0).unwrap();
        let html = BriefingGenerator::generate(&topics, "Test", date);

        assert!(html.contains("Apple &amp; Google"));
        assert!(html.contains("Test &lt;script&gt;"));
        assert!(html.contains("Point &quot;quoted&quot;"));
    }

    // ==================== CSV Generation Tests ====================

    #[test]
    fn test_generate_links_csv() {
        use crate::summarizer::Summary;

        let topics = vec![Topic {
            title: "Apple".to_string(),
            stories: vec![
                Story {
                    title: "Article 1".to_string(),
                    url: "https://a.com".to_string(),
                    created: "2026-02-01".to_string(),
                    summary: Summary::Insufficient,
                },
                Story {
                    title: "Article 2".to_string(),
                    url: "https://b.com".to_string(),
                    created: "2026-02-01".to_string(),
                    summary: Summary::Insufficient,
                },
            ],
        }];

        let csv = BriefingGenerator::generate_links_csv(&topics);

        // First row should have topic title
        assert!(csv.contains(",Apple,Article 1,,https://a.com"));
        // Second row should have blank topic
        assert!(csv.contains(",,Article 2,,https://b.com"));
    }

    // ==================== Org Mode Generation Tests ====================

    #[test]
    fn test_generate_org_mode() {
        use crate::summarizer::Summary;

        let topics = vec![Topic {
            title: "Tech".to_string(),
            stories: vec![Story {
                title: "Story Title".to_string(),
                url: "https://example.com".to_string(),
                created: "2026-02-01".to_string(),
                summary: Summary::Editorial {
                    lede: "Apple unveiled a new M5 chip at its spring event".to_string(),
                    nutgraf: "The announcement signals Apple's continued investment in custom silicon, which has reshaped the laptop and desktop market since the M1 transition in 2020.".to_string(),
                    quote: Some("\"A quote\" -- Author".to_string()),
                },
            }],
        }];

        let date = Utc.with_ymd_and_hms(2026, 2, 1, 12, 0, 0).unwrap();
        let org = BriefingGenerator::generate_org_mode(&topics, "TWiT", date);

        assert!(org.contains("#+TITLE: TWiT Briefing Book"));
        assert!(org.contains("* Tech"));
        assert!(org.contains("** Story Title"));
        assert!(org.contains("*** URL\nhttps://example.com"));
        assert!(org.contains("*** Summary"));
        assert!(org.contains("LEDE: Apple unveiled a new M5 chip"));
        assert!(org.contains("NUTGRAF: The announcement signals"));
        assert!(org.contains("\"A quote\" -- Author"));
        // Verify blank line between LEDE and NUTGRAF
        assert!(org.contains("LEDE: Apple unveiled a new M5 chip at its spring event\n\nNUTGRAF:"));
    }

    #[test]
    fn test_generate_org_mode_includes_standard_sections() {
        let topics = vec![];
        let date = Utc.with_ymd_and_hms(2026, 2, 1, 12, 0, 0).unwrap();
        let org = BriefingGenerator::generate_org_mode(&topics, "Test", date);

        assert!(org.contains("* In Other News"));
        assert!(org.contains("* Leo's Picks"));
        assert!(org.contains("* In Memoriam"));
    }
}
