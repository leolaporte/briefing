use anyhow::{Context, Result};
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub raindrop_api_token: String,
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

        Ok(Self { raindrop_api_token })
    }

    fn try_load_dotenv() {
        // Primary: env vars from shell (fish sources ~/.secrets.env via sops on startup)
        // Fallback: .env in current directory (for development)
        let _ = dotenvy::dotenv();
    }
}
