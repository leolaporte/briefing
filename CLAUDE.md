# Claude Development Notes

## Project Overview

Podcast briefing tools for TWiT, MacBreak Weekly, and Intelligent Machines podcasts. Two-stage workflow: `collect-stories` fetches and summarizes articles, `prepare-briefing` converts edited org files to HTML/CSV.

## Recent Changes (2026-01-31)

### 1. Rate Limit Handling in Clustering

**Problem:** Topic clustering would fail on rate limits, resulting in no topics being created.

**Solution:** Added retry logic with exponential backoff (matching existing summarizer logic):
- Up to 5 retry attempts
- Detects rate limits by checking for "rate_limit" or "429" in error messages
- Exponential backoff: 15s, 30s, 45s, 60s
- Falls back to chronological grouping if all retries fail
- Captures HTTP status codes in error messages for better debugging

**Files:** `crates/shared/src/clustering.rs`

### 2. Article Publication Date Extraction

**Problem:** Story dates were showing Raindrop bookmark creation date instead of article publication date.

**Solution:** Enhanced article extraction to parse publication dates from HTML metadata:
- Added `scraper` dependency for HTML parsing
- Created `ArticleContent` struct with `text` and `published_date` fields
- Extracts dates from multiple meta tag patterns:
  - `article:published_time`
  - `og:published_time`
  - `publishdate`, `publish_date`, `date`
  - `datePublished` (itemprop)
  - `<time datetime>` tags
- Formats dates as: `"Wednesday, 29 January 2026 3:17 PM"`
- Falls back to Raindrop bookmark date if no publication date found
- Updated org-mode output to include `*** Date` section
- Updated prepare-briefing parser to read Date section

**Files:**
- `crates/shared/src/extractor.rs`
- `crates/shared/src/briefing.rs`
- `crates/prepare-briefing/src/main.rs`

### 3. New Tool: prepare-briefing

**Purpose:** Standalone binary that converts manually-edited org files to HTML and CSV.

**Features:**
- Parses org-mode format (topics, stories, URLs, dates, summaries, quotes)
- Interactive file selection from `~/Documents/` (sorted by modification time)
- Or direct file specification via `--file` parameter
- Generates HTML with:
  - Two-line centered title (show name + date)
  - Collapsible topics (start collapsed with ▶ arrow)
  - Clean styling with blue accents
  - Responsive layout
- Generates CSV for Google Sheets with proper column layout
- Preserves all manual edits from org file

**Files:**
- New crate: `crates/prepare-briefing/`
- `crates/prepare-briefing/src/main.rs`
- `crates/prepare-briefing/Cargo.toml`

### 4. HTML Output Improvements

**Two-line title format:**
```
TWiT Briefing
Sunday, 2 February 2026
```

**Collapsible topics:**
- All topics start collapsed by default
- Click to expand/collapse individual topics
- Visual indicators: ▶ (collapsed) / ▼ (expanded)

**Files:** `crates/shared/src/briefing.rs`

### 5. Comprehensive Documentation

**Updated README.md with:**
- Complete workflow documentation
- Command-line options for both tools
- Usage examples
- Org-mode and HTML output format examples
- Advanced features (date extraction, rate limiting)
- Troubleshooting guide
- Development instructions
- Project structure
- Tips and best practices

**Files:** `README.md`

## Workflow

### Development Workflow
```bash
# 1. Collect stories (day before podcast)
collect-stories --show twit --days 7

# 2. Edit org file in Emacs
emacsclient ~/Documents/twit-2026-01-31.org

# 3. Generate HTML and CSV for upload
prepare-briefing --file ~/Documents/twit-2026-01-31.org

# 4. Upload to Google Docs
# - twit-2026-01-31.html
# - twit-2026-01-31-LINKS.csv
```

### Build & Install
```bash
cargo build --release --workspace
cp target/release/collect-stories ~/.local/bin/
cp target/release/prepare-briefing ~/.local/bin/
```

## Dependencies Added

- `scraper = "0.20"` - HTML parsing for metadata extraction

## Architecture Notes

### Two-Binary Design
- **collect-stories** - Heavy lifting: fetches, extracts, summarizes, clusters (uses AI APIs)
- **prepare-briefing** - Lightweight: parses org, generates HTML/CSV (no AI, runs locally)

### Separation Benefits
- Manual editing step in between (org files are human-editable)
- No need to re-run expensive AI operations after reordering/editing
- Clean separation of concerns

### Shared Library
- `crates/shared/` - Common code used by both binaries
- Models, API clients, generators, parsers
- Reduces duplication

## Known Limitations

### Google News URLs
- Google News RSS URLs (`news.google.com/rss/articles/...`) don't resolve automatically
- Workaround: Manually bookmark the actual article URL instead of Google News URL
- Future: Could add browser extension integration to resolve at bookmark time

### Rate Limits
- Claude Haiku API has rate limits
- Mitigated with: concurrency limits, delays, retry logic
- Typical run (40-50 articles): ~$0.05 cost

## Future Enhancements

### Potential Features
- [ ] Support for additional shows
- [ ] Customizable summary bullet count
- [ ] Alternative AI providers (OpenAI, Gemini)
- [ ] Browser extension for better bookmark capture
- [ ] Direct Google Docs upload via API
- [ ] Automatic publication date correction/validation
- [ ] Show-specific theming in HTML output

## Development Environment

- **Language:** Rust 2021 edition
- **Async Runtime:** Tokio
- **HTTP Client:** Reqwest
- **HTML Parsing:** Scraper, html2text
- **CLI:** Clap
- **Date/Time:** Chrono
- **Editor:** Emacs (org-mode files)
- **Platform:** Linux (CachyOS)

## Testing Notes

- Manual testing with real Raindrop.io bookmarks
- Test with 40-50 articles typical
- Verify org-mode output in Emacs
- Verify HTML rendering in browser
- Verify CSV import in Google Sheets

## Author

Leo Laporte

## Session Date

2026-01-31
