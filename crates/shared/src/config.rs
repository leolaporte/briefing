use anyhow::{Context, Result};
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub raindrop_api_token: String,
    pub anthropic_api_key: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // Try to load .env from multiple locations
        Self::try_load_dotenv();

        let raindrop_api_token = env::var("RAINDROP_TOKEN").context(
            "RAINDROP_TOKEN not found.\n\n\
                Add to ~/.secrets.env (sops-encrypted):\n  \
                RAINDROP_TOKEN=your_token_here\n\n\
                Get your Raindrop.io API token from: https://app.raindrop.io/settings/integrations",
        )?;

        let anthropic_api_key = env::var("CLAUDE_API_KEY").context(
            "CLAUDE_API_KEY not found.\n\n\
                Add to ~/.secrets.env (sops-encrypted):\n  \
                CLAUDE_API_KEY=your_key_here\n\n\
                Get your Anthropic API key from: https://console.anthropic.com/settings/keys",
        )?;

        Ok(Self {
            raindrop_api_token,
            anthropic_api_key,
        })
    }

    fn try_load_dotenv() {
        // Primary: env vars from shell (fish sources ~/.secrets.env via sops on startup)
        // Fallback: .env in current directory (for development)
        let _ = dotenvy::dotenv();
    }
}
