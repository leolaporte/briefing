use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::PathBuf;

use crate::clustering::Topic;
use crate::summarizer::Summary;

pub struct BriefingGenerator;

impl BriefingGenerator {
    fn format_date(date_str: &str) -> String {
        // Parse the ISO 8601 date string
        if let Ok(dt) = date_str.parse::<DateTime<Utc>>() {
            // Format as "Wednesday, 01/29/2026 3:17 AM"
            dt.format("%A, %m/%d/%Y %l:%M %p").to_string()
        } else {
            // Fallback to original string if parsing fails
            date_str.to_string()
        }
    }

    fn calculate_next_show_date(show_name: &str, from_date: DateTime<Utc>) -> String {
        use chrono::{Datelike, Weekday};

        let target_weekday = match show_name {
            "TWiT" => Weekday::Sun,
            "MacBreak Weekly" => Weekday::Tue,
            "Intelligent Machines" => Weekday::Wed,
            _ => Weekday::Sun, // Default to Sunday
        };

        let current_day = from_date.weekday().num_days_from_monday();
        let target_day = target_weekday.num_days_from_monday();

        let days_until_target = if current_day <= target_day {
            target_day - current_day
        } else {
            7 - (current_day - target_day)
        };

        let next_show = from_date + chrono::Duration::days(days_until_target as i64);

        // Format as "Sun, 1 February 2026"
        next_show.format("%a, %-d %B %Y").to_string()
    }

    pub fn generate(topics: &[Topic], show_name: &str, date: DateTime<Utc>) -> String {
        let mut html = String::new();

        // Format date as "Sunday, 1 February 2026"
        let formatted_date = date.format("%A, %-d %B %Y").to_string();

        // HTML header with styling
        html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
        html.push_str("  <meta charset=\"UTF-8\">\n");
        html.push_str(&format!("  <title>{} Briefing - {}</title>\n", show_name, formatted_date));
        html.push_str("  <style>\n");
        html.push_str("    body { font-family: Arial, sans-serif; max-width: 900px; margin: 40px auto; padding: 0 20px; line-height: 1.6; }\n");
        html.push_str("    h1 { color: #2c3e50; border-bottom: 3px solid #3498db; padding-bottom: 10px; text-align: center; }\n");
        html.push_str("    h1 .show-name { display: block; font-size: 1.2em; margin-bottom: 10px; }\n");
        html.push_str("    h1 .date { display: block; font-size: 0.8em; font-weight: normal; color: #555; }\n");
        html.push_str("    h2 { color: #34495e; margin: 0; padding: 10px; background-color: #ecf0f1; border-left: 4px solid #3498db; }\n");
        html.push_str("    h3 { color: #2c3e50; margin-top: 25px; }\n");
        html.push_str("    .metadata { color: #7f8c8d; font-size: 0.9em; margin: 5px 0; }\n");
        html.push_str("    .link { color: #3498db; text-decoration: none; }\n");
        html.push_str("    .link:hover { text-decoration: underline; }\n");
        html.push_str("    details.topic { margin: 40px 0 20px 0; }\n");
        html.push_str("    details.topic > summary { display: block; cursor: pointer; user-select: none; }\n");
        html.push_str("    details.topic > summary:hover h2 { background-color: #d5dbdb; }\n");
        html.push_str("    details.topic > summary h2::before { content: '▼ '; font-size: 0.8em; }\n");
        html.push_str("    details.topic:not([open]) > summary h2::before { content: '▶ '; }\n");
        html.push_str("    details.article { margin: 15px 0; padding: 10px; background-color: #f8f9fa; border-radius: 4px; }\n");
        html.push_str("    details.article > summary { display: none; }\n");
        html.push_str("    ul { margin: 10px 0; padding-left: 20px; }\n");
        html.push_str("    li { margin: 8px 0; }\n");
        html.push_str("    hr { border: none; border-top: 1px solid #ddd; margin: 30px 0; }\n");
        html.push_str("    .error { color: #e74c3c; font-style: italic; }\n");
        html.push_str("  </style>\n");
        html.push_str("</head>\n<body>\n");

        // Main title (two lines)
        html.push_str(&format!(
            "<h1><span class=\"show-name\">{} Briefing</span><span class=\"date\">{}</span></h1>\n",
            show_name,
            formatted_date
        ));

        // Topics
        for topic in topics {
            html.push_str("<details class=\"topic\">\n");
            html.push_str(&format!("  <summary><h2>{}</h2></summary>\n", Self::escape_html(&topic.title)));
            html.push_str("  <div>\n");

            for story in &topic.stories {
                html.push_str(&format!("    <h3>{}</h3>\n", Self::escape_html(&story.title)));
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
                    Summary::Success { points, quote } => {
                        html.push_str("    <details class=\"article\" open>\n");
                        html.push_str("      <summary></summary>\n");
                        if let Some(q) = quote {
                            html.push_str(&format!("      <p><em>\"{}\"</em></p>\n", Self::escape_html(q)));
                        }
                        html.push_str("      <ul>\n");
                        for point in points.iter() {
                            html.push_str(&format!("        <li>{}</li>\n", Self::escape_html(point)));
                        }
                        html.push_str("      </ul>\n");
                        html.push_str("    </details>\n");
                    }
                    Summary::Insufficient | Summary::Failed(_) => {
                        html.push_str("    <p class=\"error\">No summary available</p>\n");
                    }
                }

                html.push_str("    <hr>\n");
            }

            html.push_str("  </div>\n");
            html.push_str("</details>\n");
        }

        // Add footer section for co-hosts
        html.push_str("<hr style=\"margin: 60px 0 30px 0; border-top: 2px solid #3498db;\">\n");
        html.push_str("<h2 style=\"text-align: center; color: #2c3e50;\">Co-hosts please feel free to add stories below...</h2>\n");

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
                    csv.push_str(&format!(",,{},,{}\n", Self::escape_csv(&story.title), Self::escape_csv(&story.url)));
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
                    Summary::Success { points, quote } => {
                        // Add quote first if it exists (quote already includes quotes and attribution)
                        if let Some(q) = quote {
                            org.push_str(&format!("{}\n\n", q));
                        }
                        // Add bullet points
                        for point in points {
                            org.push_str(&format!("- {}\n", point));
                        }
                    }
                    Summary::Insufficient => {
                        org.push_str("Summary unavailable\n");
                    }
                    Summary::Failed(_) => {
                        org.push_str("Summary unavailable\n");
                    }
                }
                org.push_str("\n");
            }
        }

        // Add three empty topics at the end
        org.push_str("* Back of the Book\n\n");
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
