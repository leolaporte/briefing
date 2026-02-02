# Podcast Briefing Tools

A pair of Rust CLI tools for creating podcast briefing documents. Fetches bookmarked articles from Raindrop.io, summarizes them using Claude AI, groups related stories by topic, and generates briefing documents in multiple formats for This Week in Tech (twit), MacBreak Weekly (mbw), and Intelligent Machines (im) podcasts.

## Tools Overview

### `collect-stories`
Fetches articles from Raindrop.io, extracts content, generates AI summaries, clusters by topic, and creates an Emacs org-mode document.

### `prepare-briefing`
Converts manually-edited org-mode documents to HTML and CSV formats ready for upload to Google Docs.

---

## Features

### collect-stories

- **Raindrop.io Integration**: Fetches tagged bookmarks from configurable date ranges
- **Browser Cookie Support**: Accesses paywalled articles using Chrome/Firefox cookies
- **Complete Bookmark Inclusion**: ALL tagged bookmarks appear in output, even if extraction fails
- **Parallel Article Extraction**: Concurrent web scraping with retry logic and rate limiting
- **Publication Date Extraction**: Automatically extracts article publication dates from HTML metadata
- **AI Summarization**: 5-bullet summaries with optional quotes using Claude Haiku 4.5
- **Intelligent Topic Clustering**: Groups related articles by company or category with AI
- **Rate Limit Handling**: Automatic retry with exponential backoff for API rate limits
- **Error Logging**: Failed extractions logged to `/tmp/collect-stories-errors.log`
- **Org-Mode Output**: Clean, structured Emacs org-mode documents

### prepare-briefing

- **Org-Mode Parsing**: Reads and parses manually-edited org files
- **HTML Generation**: Beautiful, collapsible HTML briefings with two-line titles
- **CSV Export**: Links spreadsheet formatted for Google Sheets
- **Interactive File Selection**: Lists available org files sorted by modification time
- **Preserves Edits**: Works with your manually reordered and edited content

---

## Installation

### Prerequisites

- **Rust toolchain** - Install from [rustup.rs](https://rustup.rs)
- **Raindrop.io API token** - Get from [app.raindrop.io/settings/integrations](https://app.raindrop.io/settings/integrations)
- **Anthropic Claude API key** - Get from [console.anthropic.com/settings/keys](https://console.anthropic.com/settings/keys)

### Build and Install

```bash
cd ~/Projects/collect-stories
cargo build --release --workspace
cp target/release/collect-stories ~/.local/bin/
cp target/release/prepare-briefing ~/.local/bin/
```

### Configure API Keys

Create environment file at `~/.config/podcast-briefing/.env`:

```bash
mkdir -p ~/.config/podcast-briefing
cat > ~/.config/podcast-briefing/.env << 'EOF'
RAINDROP_API_TOKEN=your_raindrop_token_here
ANTHROPIC_API_KEY=your_anthropic_api_key_here
EOF
```

The tools search for `.env` in this order:
1. Current directory (for development)
2. `~/.config/podcast-briefing/.env` (recommended)
3. `~/.env` (home directory)
4. System environment variables

---

## Complete Workflow

### Step 1: Collect Stories During the Week

Tag bookmarks in Raindrop.io with show-specific tags:
- twit (This Week in Tech): `#twit`
- mbw (MacBreak Weekly): `#mbw`
- im (Intelligent Machines): `#im`

### Step 2: Generate Initial Briefing

Day before podcast recording, run `collect-stories`:

```bash
# Interactive mode (prompts for show selection)
collect-stories

# Or specify show and date range
collect-stories --show twit --days 7
```

**What it does:**
1. 📚 Fetches bookmarks with show tag from Raindrop.io
2. 🌐 Extracts article content and publication dates in parallel
3. 🤖 Summarizes each article using Claude Haiku (5 bullets + optional quote)
4. 🔗 Groups articles by company or topic using AI clustering
5. 📝 Generates org-mode document in `~/Documents/`

**Output:** `~/Documents/{show}-{date}.org`

### Step 3: Manual Editing

Open the org file in Emacs and edit as needed:

```bash
emacsclient ~/Documents/twit-2026-01-31.org
```

**Common edits:**
- Reorder stories by importance
- Remove irrelevant or duplicate stories
- Edit summaries for clarity
- Reorganize topics
- Add custom notes or sections

### Step 4: Generate HTML and CSV

After editing, run `prepare-briefing`:

```bash
# Interactive mode (shows list of .org files)
prepare-briefing

# Or specify the file directly
prepare-briefing --file ~/Documents/twit-2026-01-31.org
```

**What it does:**
1. 📖 Reads your edited org-mode file
2. 🔍 Parses topics, stories, and summaries
3. 📝 Generates HTML briefing with collapsible topics
4. 📊 Generates CSV with links for spreadsheet

**Outputs:**
- `~/Documents/twit-2026-01-31.html` - HTML briefing for Google Docs
- `~/Documents/twit-2026-01-31-LINKS.csv` - Links spreadsheet

Upload these files to Google Docs for your producers and hosts.

---

## collect-stories Usage

### Command-Line Options

```bash
collect-stories [OPTIONS]
```

**Options:**
- `--show <slug>` - Show to collect for: `twit`, `mbw`, or `im`
  - Default: Interactive prompt
- `--days <num>` - Number of days to look back for bookmarks
  - Default: 7

### Examples

```bash
# Interactive mode - prompts for show selection
collect-stories

# Collect last week's twit (This Week in Tech) stories
collect-stories --show twit

# Collect last 2 weeks of mbw (MacBreak Weekly) stories
collect-stories --show mbw --days 14

# Collect last 3 days of im (Intelligent Machines) stories
collect-stories --show im --days 3
```

### Output Format (Org-Mode)

```org
#+TITLE: TWiT Briefing Book
#+DATE: Sunday, 2 February 2026

* Apple

** Apple unveils new MacBook Pro

*** URL
https://example.com/macbook-pro-2026

*** Summary
"This is the most powerful MacBook we've ever created" -- Tim Cook
- New M5 chip delivers 40% performance improvement over M4
- Revolutionary Liquid Retina XDR display with 120Hz ProMotion
- Enhanced battery life provides up to 22 hours of video playback
- Redesigned thermal architecture enables sustained peak performance
- Starting price remains unchanged at $1,999

* Google

** Google releases Gemini 2.0 update

*** URL
https://example.com/gemini-2-0

*** Summary
- Gemini 2.0 introduces multimodal capabilities across text, images, and video
- New reasoning engine improves accuracy on complex queries by 35%
- Integration with Google Workspace enables AI-powered document analysis
- Enhanced privacy controls allow users to opt out of training data
- Available now to Google One subscribers, free tier coming in March

* Back of the Book

* Leo's Picks

* In Memoriam
```

**Structure:**
- Level 1 (`*`) - Topic names (company/category) + placeholder sections
- Level 2 (`**`) - Article titles
- Level 3 (`***`) - URL and Summary sections
- Quotes appear first (if extracted from article)
- Summary bullets use standard org-mode list format (`-`)

**Using in Emacs:**
- `TAB` - Fold/unfold sections
- `C-c C-n` - Next heading
- `C-c C-p` - Previous heading
- `C-c C-e` - Export to other formats

---

## prepare-briefing Usage

### Command-Line Options

```bash
prepare-briefing [OPTIONS]
```

**Options:**
- `--file <path>` - Path to org-mode file to convert
  - Default: Interactive file selection from `~/Documents/`

### Examples

```bash
# Interactive mode - lists available org files
prepare-briefing

# Convert specific file
prepare-briefing --file ~/Documents/twit-2026-01-31.org
```

### Interactive File Selection

When run without `--file`, shows numbered list of org files:

```
Available org files:

  1) twit-2026-01-31.org (modified: 2026-01-31 15:33)
  2) mbw-2026-01-28.org (modified: 2026-01-28 14:22)
  3) im-2026-01-27.org (modified: 2026-01-27 10:15)

Select file (1-3):
```

Files are sorted by modification time (newest first).

### HTML Output Format

**Title Format (Two Lines):**
```
TWiT Briefing
Sunday, 2 February 2026
```

**Features:**
- Clean, professional styling with Arial font
- Centered two-line title (show name larger, date smaller and gray)
- Collapsible topics (click to expand/collapse)
- Blue accents and borders
- Responsive layout (max-width 900px, centered)
- Article metadata (links, dates) styled consistently
- Quote formatting (italicized)
- Bullet points for summaries

**Topics Start Collapsed:**
All topics begin in collapsed state (▶ arrow). Click any topic to expand (▼ arrow) and view stories.

### CSV Output Format

Formatted for Google Sheets with columns:
- Column A: Empty
- Column B: Topic title (first article only)
- Column C: Article title
- Column D: Empty
- Column E: Article URL

Example:
```csv
,Apple,Apple unveils new MacBook Pro,,https://example.com/macbook-pro
,,Apple announces new AI features,,https://example.com/ai-features
,,,,
,Google,Google releases Gemini 2.0 update,,https://example.com/gemini-2-0
,,,,
```

Blank rows separate topics for easy reading.

---

## Advanced Features

### Browser Cookie Support

`collect-stories` automatically loads browser cookies to access paywalled content:

**Supported Browsers:**
- **Firefox** (recommended)
  - Cookie database: `~/.mozilla/firefox/*/cookies.sqlite`
  - Uses Unix epoch for timestamps
  - Works reliably on Linux
- **Chrome/Chromium** (limited support)
  - Cookie database: `~/.config/google-chrome/Default/Cookies`
  - ⚠️ **Note:** Chrome encrypts cookies on Linux - see troubleshooting below

**How it works:**
1. Loads cookies from browser database before fetching articles
2. Filters expired cookies automatically
3. Applies relevant cookies to each article request
4. Enables access to Forbes, WSJ, NYT, and other paywalled sites you're logged into

**Requirements:**
- You must be logged into the site in your browser
- Browser must store persistent cookies (not incognito/private mode)
- Works with sites you have active subscriptions to

**Chrome Encryption Issue (Linux):**
Chrome/Chromium on Linux encrypts cookie values using the system keyring. The tool currently cannot decrypt these cookies. **Workaround:** Use Firefox for accessing paywalled sites, or use a browser extension to export cookies manually.

**Privacy note:** Cookies are only read locally and used for article fetching. They are never uploaded or shared.

### Complete Bookmark Preservation

**All bookmarks with the proper tag appear in the org file**, regardless of extraction success:

**For successfully extracted articles:**
- Full AI-generated summary with 5 bullet points
- Optional quote from article
- Extracted publication date

**For failed extractions:**
- Title and URL from Raindrop bookmark
- Bookmark creation date
- "Summary not available" placeholder
- Error logged to `/tmp/collect-stories-errors.log`

**Why this matters:**
- No bookmarks are lost due to paywalls or scraping issues
- You can manually research failed articles later
- Complete audit trail of all tagged content
- Easy to identify which sites need alternative access

### Publication Date Extraction

`collect-stories` automatically extracts article publication dates from HTML metadata:

**Supported meta tags:**
- `article:published_time`
- `og:published_time`
- `publishdate`, `publish_date`, `date`
- `datePublished` (itemprop)
- `<time datetime>` tags

**Date format:**
- Parses ISO 8601 and standard date formats
- Displays as: `"Wednesday, 29 January 2026 3:17 PM"`
- Falls back to Raindrop bookmark date if not found

### Rate Limit Handling

Both tools automatically handle API rate limits:

**Summarization (Claude Haiku):**
- Concurrency limited to 2 parallel requests
- 500ms delay between successful requests
- Up to 5 retry attempts on rate limit
- Exponential backoff: 15s, 30s, 45s, 60s

**Clustering (Claude Haiku):**
- Up to 5 retry attempts on rate limit
- Exponential backoff: 15s, 30s, 45s, 60s
- Falls back to chronological grouping if all retries fail

**Article Extraction:**
- Concurrency limited to 10 parallel requests
- Up to 3 retry attempts per article
- Exponential backoff: 500ms, 1s, 2s

### Error Handling

The tools gracefully handle:

**Missing API keys:**
```
Error: RAINDROP_API_TOKEN not found
Create ~/.config/podcast-briefing/.env with your API tokens
```

**No bookmarks found:**
```
No bookmarks found with tag #twit in the past 7 days.
```

**Paywalled/unreachable articles:**
- All bookmarks included in org file (never lost)
- Successfully extracted: Full AI summary
- Failed extractions: "Summary not available" placeholder
- Progress shown: "✓ Successfully extracted content from 42/50 articles"
- Errors logged to `/tmp/collect-stories-errors.log` with timestamps

**Viewing extraction errors:**
```bash
# Recent errors
tail -f /tmp/collect-stories-errors.log

# All errors from this session
cat /tmp/collect-stories-errors.log
```

**Rate limits:**
```
Rate limit hit during clustering, waiting 15s before retry 2 of 5...
```

**Failed clustering:**
```
Clustering failed after 5 attempts: ..., using chronological fallback
✓ Organized into 1 topics
```

---

## Cost Estimate

**Claude Haiku 4.5 API:**
- ~$0.001 per article (summarization + clustering)
- For 100 articles: ~$0.10 total
- Typical weekly run (40-50 articles): ~$0.05

**Note:** Costs may vary based on article length and API pricing.

---

## Troubleshooting

### Problem: `RAINDROP_API_TOKEN not found`

**Solution:** Create `~/.config/podcast-briefing/.env` with your API tokens:

```bash
mkdir -p ~/.config/podcast-briefing
cat > ~/.config/podcast-briefing/.env << 'EOF'
RAINDROP_API_TOKEN=your_token_here
ANTHROPIC_API_KEY=your_api_key_here
EOF
```

### Problem: No bookmarks found

**Solution:** Check that you've tagged bookmarks in Raindrop.io with the correct tag (`#twit`, `#mbw`, or `#im`). Tags are case-sensitive and must include the `#` symbol.

### Problem: Many articles failing to extract

**Good news:** All bookmarks are preserved in the org file, even if extraction fails!

**Common extraction failures:**
- **Paywalled sites** (WSJ, NYT, Forbes, etc.)
  - Solution: Log into the site in Chrome or Firefox before running collect-stories
  - The tool uses your browser cookies to access paywalled content
- **Anti-bot protection** (Cloudflare, Imperva, etc.)
  - Some sites block automated scrapers
  - These will show "Summary not available" in org file
- **JavaScript-required sites**
  - Sites that load content dynamically may not work
  - Consider bookmarking the direct article URL instead of aggregator links
- **Restrictive robots.txt**
  - Some sites block web crawlers entirely

**Check error log for details:**
```bash
tail /tmp/collect-stories-errors.log
```

**Manual fallback:** Failed articles appear in your org file with title and URL, so you can manually review them before the show.

### Problem: Rate limits from Claude API

**Solution:** The tools automatically handle rate limits with exponential backoff. If you see rate limit messages, just wait - the tools will retry automatically. For persistent issues:
- Reduce parallel requests (edit source code)
- Space out your runs
- Check your API quota at console.anthropic.com

### Problem: HTML topics won't expand/collapse

**Solution:** Ensure you're opening the HTML file in a modern browser (Chrome, Firefox, Safari, Edge). The `<details>` element is supported in all modern browsers.

### Problem: CSV not formatting correctly in Google Sheets

**Solution:** When uploading to Google Sheets:
1. File → Import
2. Choose "Comma" as separator
3. Ensure "Convert text to numbers" is UNCHECKED
4. URLs should appear in column E

---

## Development

### Build Commands

```bash
# Build entire workspace (debug)
cargo build --workspace

# Build entire workspace (release)
cargo build --release --workspace

# Build specific binary
cargo build --release -p collect-stories
cargo build --release -p prepare-briefing

# Install to ~/.local/bin/
cp target/release/collect-stories ~/.local/bin/
cp target/release/prepare-briefing ~/.local/bin/
```

### Run in Development

```bash
# Run collect-stories from source
cargo run -p collect-stories -- --show twit

# Run prepare-briefing from source
cargo run -p prepare-briefing -- --file ~/Documents/twit-2026-01-31.org
```

### Testing

```bash
# Run all tests
cargo test --workspace

# Run tests for specific crate
cargo test -p shared
cargo test -p collect-stories
cargo test -p prepare-briefing
```

### Code Quality

```bash
# Check for errors without building
cargo check --workspace

# Run linter
cargo clippy --workspace

# Format code
cargo fmt --workspace
```

---

## Project Structure

```
collect-stories/
├── Cargo.toml                    # Workspace manifest
├── README.md                     # This file
└── crates/
    ├── collect-stories/          # Main binary: fetch & generate org-mode
    │   ├── Cargo.toml
    │   └── src/
    │       └── main.rs           # CLI entry point, workflow orchestration
    │
    ├── prepare-briefing/         # Secondary binary: convert org to HTML/CSV
    │   ├── Cargo.toml
    │   └── src/
    │       └── main.rs           # Org parser, HTML/CSV generator
    │
    └── shared/                   # Shared library
        ├── Cargo.toml
        └── src/
            ├── lib.rs            # Public API exports
            ├── config.rs         # Environment configuration
            ├── raindrop.rs       # Raindrop.io API client
            ├── extractor.rs      # Web scraping + date extraction
            ├── summarizer.rs     # Claude AI summarization
            ├── clustering.rs     # Topic clustering with retry logic
            ├── briefing.rs       # Org-mode/HTML/CSV generation
            ├── models.rs         # Shared data structures
            └── io.rs             # File I/O utilities
```

### Key Dependencies

- **reqwest** - HTTP client for API calls and web scraping
- **tokio** - Async runtime for parallel operations
- **serde/serde_json** - Serialization for API requests/responses
- **anyhow** - Error handling
- **chrono** - Date/time parsing and formatting
- **html2text** - HTML to text conversion
- **scraper** - HTML parsing for metadata extraction
- **rusqlite** - Browser cookie database access (Chrome/Firefox)
- **cookie_store** - Cookie management and parsing
- **url** - URL parsing for cookie domain matching
- **clap** - Command-line argument parsing
- **dirs** - Platform-specific directory paths

---

## Tips and Best Practices

### Tagging in Raindrop.io

- Tag articles throughout the week as you find them
- Use consistent tags: `#twit`, `#mbw`, `#im`
- Add tags immediately when bookmarking for better organization
- You can tag the same article with multiple show tags

### Editing Org Files

- Remove duplicate or similar stories before publishing
- Reorder topics by importance (breaking news first, misc last)
- Edit summaries to match your speaking style
- Add your own notes or talking points in the org file
- Use org-mode folding to focus on one topic at a time

### Preparing for Upload

- Review HTML in browser before uploading to Google Docs
- Check CSV formatting in a spreadsheet app first
- Keep a local backup of edited org files
- Name files consistently for easy tracking

### Performance

- Run `collect-stories` during off-peak hours if you have many articles
- The tool uses parallel processing for speed but respects rate limits
- Expect ~1-2 minutes for 50 articles (including AI processing)
- `prepare-briefing` is very fast (<1 second) since it's local only

---

## License

MIT

## Author

Leo Laporte
