use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::rsc;

const HOMEPAGE: &str = "https://artificialanalysis.ai/";
/// Context windows and cache-write prices are not part of the homepage payload;
/// AA publishes both on the models leaderboard, which we merge in by slug.
const MODELS_LEADERBOARD: &str = "https://artificialanalysis.ai/leaderboards/models";

#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default, alias = "modelCreators", alias = "creator")]
    pub model_creators: Option<Creator>,
    #[serde(default, alias = "intelligenceIndex")]
    pub intelligence_index: Option<f64>,
    #[serde(default, rename = "timescaleData")]
    pub timescale: Option<Timescale>,
    /// AA's headline blended price at 7:2:1 cache:input:output, USD per 1M tokens.
    #[serde(default, alias = "price1mBlended7To2To1")]
    pub price_1m_blended: Option<f64>,
    /// The same blend with no cache hits, 3:1 input:output, USD per 1M tokens.
    #[serde(default, alias = "price1mBlended0To3To1")]
    pub price_1m_blended_no_cache: Option<f64>,
    #[serde(default, alias = "price1mInputTokens")]
    pub price_1m_input_tokens: Option<f64>,
    #[serde(default, alias = "price1mOutputTokens")]
    pub price_1m_output_tokens: Option<f64>,
    #[serde(default, alias = "cacheHitPrice")]
    pub cache_hit_price: Option<f64>,
    /// Price per 1M tokens to write to the provider's prompt cache. AA only
    /// publishes it for providers that charge for writes.
    #[serde(default, alias = "cacheWritePrice")]
    pub cache_write_price: Option<f64>,
    /// Share of the input price saved on cache hits, as AA's "Cache Discount"
    /// (0.9 = 90% cheaper). AA ships it directly; it is not always
    /// `1 - cache_hit_price / price_1m_input_tokens`.
    #[serde(default, alias = "cacheHitDiscountPercent")]
    pub cache_hit_discount: Option<f64>,
    #[serde(default, alias = "contextWindowTokens")]
    pub context_window_tokens: Option<u64>,
    #[serde(default, alias = "releaseDate")]
    pub release_date: Option<String>,
    #[serde(default, alias = "isOpenWeights")]
    pub is_open_weights: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Creator {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Timescale {
    #[serde(default, alias = "medianOutputSpeed")]
    pub median_output_speed: Option<f64>,
}

impl Model {
    pub fn provider(&self) -> &str {
        self.model_creators
            .as_ref()
            .map(|c| c.name.as_str())
            .unwrap_or("?")
    }

    pub fn provider_color(&self) -> Option<&str> {
        self.model_creators
            .as_ref()
            .and_then(|c| c.color.as_deref())
    }

    pub fn speed(&self) -> Option<f64> {
        self.timescale.as_ref().and_then(|t| t.median_output_speed)
    }

    /// Returns the human-readable identifier (slug) or falls back to id.
    pub fn display_id(&self) -> &str {
        self.slug.as_deref().unwrap_or(&self.id)
    }

    /// The homepage ships two records per model — a full one and a summary one.
    /// Keep the first record's scalars and fill whatever it lacks from the rest.
    fn merge_missing(&mut self, other: &Self) {
        self.slug = self.slug.clone().or_else(|| other.slug.clone());
        self.model_creators = self
            .model_creators
            .clone()
            .or_else(|| other.model_creators.clone());
        self.intelligence_index = self.intelligence_index.or(other.intelligence_index);
        self.timescale = self.timescale.clone().or_else(|| other.timescale.clone());
        self.price_1m_blended = self.price_1m_blended.or(other.price_1m_blended);
        self.price_1m_blended_no_cache = self
            .price_1m_blended_no_cache
            .or(other.price_1m_blended_no_cache);
        self.price_1m_input_tokens = self.price_1m_input_tokens.or(other.price_1m_input_tokens);
        self.price_1m_output_tokens = self.price_1m_output_tokens.or(other.price_1m_output_tokens);
        self.cache_hit_price = self.cache_hit_price.or(other.cache_hit_price);
        self.cache_write_price = self.cache_write_price.or(other.cache_write_price);
        self.cache_hit_discount = self.cache_hit_discount.or(other.cache_hit_discount);
        self.context_window_tokens = self.context_window_tokens.or(other.context_window_tokens);
        self.release_date = self
            .release_date
            .clone()
            .or_else(|| other.release_date.clone());
        self.is_open_weights = self.is_open_weights.or(other.is_open_weights);
    }
}

pub fn fetch() -> Result<Vec<Model>> {
    let html = rsc::fetch_text_retry(HOMEPAGE)?;
    let mut models = parse(&html)?;
    match fetch_leaderboard() {
        Ok(rows) => apply_leaderboard(&mut models, &rows),
        Err(e) => {
            eprintln!("warning: leaderboard extras unavailable from artificialanalysis.ai ({e:#})")
        }
    }
    Ok(models)
}

pub fn parse(html: &str) -> Result<Vec<Model>> {
    let stream = rsc::extract_stream(html)?;
    let mut out: Vec<Model> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut parse_failures = 0u32;

    for span in rsc::innermost_objects_with(&stream, "\"intelligenceIndex\":") {
        let Ok(model) = serde_json::from_str::<Model>(span) else {
            parse_failures += 1;
            continue;
        };
        if model.id.is_empty() {
            continue;
        }
        match seen.get(&model.id) {
            Some(&idx) => out[idx].merge_missing(&model),
            None => {
                seen.insert(model.id.clone(), out.len());
                out.push(model);
            }
        }
    }

    if parse_failures > 0 {
        eprintln!("warning: {parse_failures} model(s) failed to parse from artificialanalysis.ai");
    }
    if out.is_empty() {
        return Err(anyhow!("no model records found in artificialanalysis.ai"));
    }
    Ok(out)
}

/// Fields the homepage payload does not carry at all, keyed by slug on AA's
/// models leaderboard.
#[derive(Debug, Clone, Deserialize)]
struct LeaderboardRow {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default, alias = "contextWindowTokens")]
    context_window_tokens: Option<u64>,
    #[serde(default, alias = "cacheWritePrice")]
    cache_write_price: Option<f64>,
}

fn fetch_leaderboard() -> Result<HashMap<String, LeaderboardRow>> {
    let html = rsc::fetch_text_retry(MODELS_LEADERBOARD)?;
    parse_leaderboard(&html)
}

fn parse_leaderboard(html: &str) -> Result<HashMap<String, LeaderboardRow>> {
    let stream = rsc::extract_stream(html)?;
    let mut rows = HashMap::new();
    for span in rsc::innermost_objects_with(&stream, "\"contextWindowTokens\":") {
        let Ok(entry) = serde_json::from_str::<LeaderboardRow>(span) else {
            continue;
        };
        if let Some(slug) = entry.slug.clone() {
            rows.insert(slug, entry);
        }
    }
    if rows.is_empty() {
        return Err(anyhow!("no model rows found in artificialanalysis.ai"));
    }
    Ok(rows)
}

fn apply_leaderboard(models: &mut [Model], rows: &HashMap<String, LeaderboardRow>) {
    for model in models {
        let Some(row) = model.slug.as_deref().and_then(|slug| rows.get(slug)) else {
            continue;
        };
        if model.context_window_tokens.is_none() {
            model.context_window_tokens = row.context_window_tokens;
        }
        if model.cache_write_price.is_none() {
            model.cache_write_price = row.cache_write_price;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(models: &'a [Model], id: &str) -> &'a Model {
        models
            .iter()
            .find(|m| m.display_id() == id)
            .unwrap_or_else(|| panic!("fixture should include {id}"))
    }

    #[test]
    fn live_fetch_when_enabled() {
        if std::env::var("LLMPK_LIVE").is_err() {
            return;
        }
        let models = fetch().expect("live artificialanalysis fetch");
        assert!(models.len() > 10);
        assert!(
            models
                .iter()
                .filter(|m| m.context_window_tokens.is_some())
                .count()
                > 10,
            "leaderboard context windows should be merged in"
        );
        assert!(
            models.iter().any(|m| m.cache_write_price.is_some()),
            "leaderboard cache write prices should be merged in"
        );
    }

    #[test]
    fn parses_committed_fixture() {
        let html = include_str!("../tests/fixtures/aa_homepage.html");
        let models = parse(html).expect("parse committed fixture");
        assert!(
            models.len() >= 2,
            "expected >=2 models, got {}",
            models.len()
        );

        let claude = find(&models, "claude-sonnet-4");
        assert_eq!(claude.provider_color(), Some("#cc785c"));
        assert_eq!(claude.price_1m_blended, Some(2.31));
        assert_eq!(claude.price_1m_blended_no_cache, Some(6.0));
        assert_eq!(claude.price_1m_input_tokens, Some(3.0));
        assert_eq!(claude.price_1m_output_tokens, Some(15.0));
        assert_eq!(claude.cache_hit_price, Some(0.3));
        assert_eq!(claude.cache_hit_discount, Some(0.9));
    }

    #[test]
    fn summary_record_does_not_shadow_priced_record() {
        // The homepage ships a summary record and a full record per model, in
        // either order; the fixture puts the summary one first for GPT-4.1.
        let html = include_str!("../tests/fixtures/aa_homepage.html");
        let models = parse(html).expect("parse committed fixture");
        assert_eq!(models.len(), 3);

        let gpt = find(&models, "gpt-4.1");
        assert_eq!(gpt.name, "GPT-4.1");
        assert_eq!(gpt.speed(), Some(95.2));
        assert_eq!(gpt.price_1m_blended, Some(1.55));
        assert_eq!(gpt.cache_hit_discount, Some(0.75));
    }

    #[test]
    fn leaderboard_fills_context_and_cache_write_prices() {
        let html = include_str!("../tests/fixtures/aa_homepage.html");
        let mut models = parse(html).expect("parse committed fixture");
        let rows = parse_leaderboard(include_str!("../tests/fixtures/aa_leaderboard.html"))
            .expect("parse committed leaderboard fixture");

        apply_leaderboard(&mut models, &rows);

        let claude = find(&models, "claude-sonnet-4");
        assert_eq!(claude.context_window_tokens, Some(200_000));
        assert_eq!(claude.cache_write_price, Some(3.75));

        // On the leaderboard, but AA publishes no cache-write price for it.
        let gpt = find(&models, "gpt-4.1");
        assert_eq!(gpt.context_window_tokens, Some(1_047_576));
        assert_eq!(gpt.cache_write_price, None);

        // Absent from the leaderboard entirely.
        let gemini = find(&models, "gemini-2.5-flash");
        assert_eq!(gemini.context_window_tokens, None);
        assert_eq!(gemini.cache_write_price, None);
    }

    #[test]
    fn parses_fixture_when_provided() {
        let Ok(path) = std::env::var("LLMPK_HOMEPAGE_FIXTURE") else {
            return;
        };
        let html = std::fs::read_to_string(&path).expect("fixture read");
        let models = parse(&html).expect("parse");
        assert!(
            models.len() >= 10,
            "expected >=10 models, got {}",
            models.len()
        );
    }
}
