// Public modules
pub mod briefing;
pub mod clustering;
pub mod config;
pub mod extractor;
pub mod io;
pub mod models;
pub mod raindrop;
pub mod summarizer;

// Re-export commonly used types
pub use clustering::{Story, Topic, TopicClusterer};
pub use config::Config;
pub use extractor::{ArticleContent, ContentExtractor};
pub use io::{get_default_stories_dir, list_story_files, load_stories, save_stories};
pub use models::{BriefingData, ShowInfo};
pub use raindrop::RaindropClient;
pub use summarizer::{ClaudeSummarizer, Summary};
