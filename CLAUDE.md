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

## Recent Changes (2026-02-01)

### Browser Cookie Support for Paywalled Sites

**Problem:** Articles behind paywalls (Forbes, WSJ, NYT, etc.) were failing to fetch even when user is logged in via browser.

**Solution:** Implemented browser cookie loading to access authenticated content:

**Key Implementation Details:**
- **Chrome Cookies:** Uses Windows FILETIME epoch (microseconds since Jan 1, 1601)
  - Timestamp calculation: `(unix_timestamp + 11_644_473_600) * 1_000_000`
  - Database: `~/.config/google-chrome/Default/Cookies`
  - Table: `cookies` with `host_key`, `expires_utc` columns

- **Firefox Cookies:** Uses Unix epoch (seconds since Jan 1, 1970)
  - Timestamp calculation: `unix_timestamp` (in seconds)
  - Database: `~/.mozilla/firefox/*/cookies.sqlite`
  - Table: `moz_cookies` with `host`, `expiry` columns

**Loading Strategy:**
1. Try Chrome/Chromium first (most commonly used)
2. Fall back to Firefox if Chrome unavailable or has 0 cookies
3. Filter out expired cookies using correct epoch for each browser
4. Display count of loaded cookies on startup

**Critical Bug Fix:**
- Initial implementation loaded 0 cookies due to incorrect timestamp format
- Was using: `chrono::Utc::now().timestamp() * 1_000_000` (Unix microseconds)
- Fixed to: `(chrono::Utc::now().timestamp() + 11_644_473_600) * 1_000_000` (Chrome FILETIME)
- Now successfully loads 187 cookies from Chrome

**Files Modified:**
- **collect-stories:**
  - `crates/shared/src/cookies.rs` (new) - Cookie loading logic
  - `crates/shared/src/extractor.rs` - Integration with reqwest
  - `Cargo.toml` - Added dependencies: rusqlite, cookie_store, url

- **beatcheck:**
  - `src/services/content_fetcher.rs` - Added Chrome support + Firefox fallback
  - Previously only supported Firefox, now tries Chrome first

**GitHub Actions:**
- CI workflow: ✅ Passing (format, build, check, clippy, test)
- Build Binaries workflow: ⚠️ Has outdated binary name "podcast-briefing" (non-critical)

**Commits:**
- `21d7b20` - feat: add browser cookie support for Chrome and Firefox
- `8a1283f` - chore: fix formatting and clippy warnings
- `ff05e75` - fix: resolve clippy warnings for CI

**Dependencies Added:**
- `rusqlite = "0.32"` - SQLite database access for cookie stores
- `cookie_store = "0.21"` - Cookie management and parsing
- `url = "2.5"` - URL parsing for cookie domain association

## Session Dates

- 2026-01-31 - Initial development and prepare-briefing tool
- 2026-02-01 - Browser cookie support implementation
- 2026-02-02 - Test coverage (55 tests) and automation setup

## Recent Changes (2026-02-02)

### Test Coverage

Added 55 tests across the workspace:
- **prepare-briefing**: 11 tests (org-mode parsing, show slug extraction)
- **shared/briefing**: 20 tests (HTML/CSV/org generation, escaping, date formatting)
- **shared/extractor**: 9 tests (date parsing, published date extraction)
- **shared/io**: 5 tests (save/load stories, error handling)
- **shared/models**: 5 tests (ShowInfo, BriefingData serialization)

Run tests with: `cargo test`

### Automated Daily Briefing

Systemd timer runs daily at 6pm Pacific to generate and upload briefings.

**Show Schedule:**
| Show | Airs | Ends |
|------|------|------|
| TWiT | Sunday | 6pm Pacific |
| MacBreak Weekly | Tuesday | 3pm Pacific |
| Intelligent Machines | Wednesday | 6pm Pacific |

**Automation:**
- Runs daily at 6pm Pacific
- Processes ALL THREE shows on every run
- Per-show lookback: only collects stories since that show's previous episode ended
- Uploads to show-specific folders: `/Briefings/{twit,mbw,im}/index.html`

**Files (in ~/Sync/dotfiles/cachyos/sway/):**
- `scripts/podcast-briefing.sh` - Main automation script
- `systemd/podcast-briefing.service` - Systemd service
- `systemd/podcast-briefing.timer` - Daily 6pm trigger

**Commands:**
```bash
# Check timer status
systemctl --user status podcast-briefing.timer

# Manual run
systemctl --user start podcast-briefing.service

# View logs
tail -f /tmp/podcast-briefing.log
```

**Key Implementation Details:**
- Credentials stored in `~/.config/podcast-briefing/.env` (chmod 600)
- WebDAV uploads to Fastmail:
  - `https://myfiles.fastmail.com/Briefings/twit/index.html`
  - `https://myfiles.fastmail.com/Briefings/mbw/index.html`
  - `https://myfiles.fastmail.com/Briefings/im/index.html`
- Must use full binary paths (`$HOME/.local/bin/collect-stories`) since `~/.local/bin` is not in systemd's PATH
- `Persistent=true` in timer ensures runs happen even after sleep/wake
