//! In-process stand-ins for every outside service a tool reaches: what
//! [`crate::deps::Deps::fake`] wires, and what the tool tests drive. None of
//! them touches the network.

use std::sync::Mutex;

use async_trait::async_trait;
use teeny_market::community::SearchError;
use teeny_market::gmgn::RankedTrader;
use teeny_market::market::{FlowSummary, FlowWindow, MarketData, WalletActivity};
use teeny_market::news::{NewsHit, NewsQuery, NewsSearch};
use teeny_market::types::{Candle, HolderSnapshot, PriceChange, Timeframe, TokenOverview, Txns};
use teeny_market::{MarketError, Result as MarketResult};
use teeny_media::giphy::{Gif, GifSource};
use teeny_media::types::{
    GeneratedImage, GeneratedVideo, ImageAspect, ImageGenerator, ImageInput, ImageReader, MimeType,
    VideoGenerator, VideoRequest,
};

/// The token the legacy market-tools test used.
pub const TOKEN: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";

/// A market answering with the legacy market-tools test's numbers: 60
/// hourly bars climbing gently, a $730.7K token, a short flow window.
pub struct FakeMarket {
    /// How many candles a series has.
    pub bars: usize,
}

impl Default for FakeMarket {
    fn default() -> Self {
        Self { bars: 60 }
    }
}

/// `count` hourly bars, as the legacy test built them.
pub fn candles(count: usize) -> Vec<Candle> {
    (0..count)
        .map(|index| {
            let step = index as f64 * 0.000_001;
            Candle {
                time: 1_700_000_000 + index as i64 * 3_600,
                open: 0.0007 + step,
                high: 0.000_71 + step,
                low: 0.000_69 + step,
                close: 0.000_705 + step,
                volume: 100.0,
            }
        })
        .collect()
}

#[async_trait]
impl MarketData for FakeMarket {
    async fn overview(&self, address: &str) -> MarketResult<TokenOverview> {
        Ok(TokenOverview {
            address: address.to_owned(),
            chain: "solana".into(),
            name: Some("Tiny Humans AI".into()),
            symbol: Some("TINY".into()),
            price_usd: 0.000_747,
            market_cap_usd: Some(730_709.0),
            fdv_usd: None,
            liquidity_usd: Some(93_428.0),
            volume24h_usd: Some(10_134.0),
            price_change: PriceChange {
                h24: Some(17.38),
                ..PriceChange::default()
            },
            txns24h: Some(Txns {
                buys: 79.0,
                sells: 75.0,
            }),
            pool_address: Some("GEF9".into()),
            pool_name: None,
            pair_created_at: None,
            source: "dexscreener".into(),
        })
    }

    async fn candles(
        &self,
        _address: &str,
        _timeframe: Timeframe,
        limit: u32,
    ) -> MarketResult<Vec<Candle>> {
        Ok(candles(self.bars.min(limit as usize)))
    }

    async fn holders(&self, _address: &str) -> MarketResult<HolderSnapshot> {
        Err(MarketError::data("no holder source in the fake"))
    }

    async fn flow(&self, _address: &str, _min_volume_usd: f64) -> MarketResult<FlowSummary> {
        Ok(FlowSummary {
            window: FlowWindow {
                from: "a".into(),
                to: "b".into(),
                trades: 24,
            },
            buy_volume_usd: 6_344.0,
            sell_volume_usd: 2_687.0,
            net_usd: 3_657.0,
            unique_wallets: 11,
            buyers: 7,
            sellers: 5,
            top_buyers: vec![WalletActivity {
                wallet: "WalletAAAAAAAAAA".into(),
                label: None,
                buys: 2,
                sells: 0,
                bought_usd: 1_600.0,
                sold_usd: 0.0,
                net_usd: 1_600.0,
                first_seen: "a".into(),
                last_seen: "b".into(),
            }],
            top_sellers: Vec::new(),
            known: Vec::new(),
        })
    }

    async fn wallet(
        &self,
        _address: &str,
        _wallet: &str,
        _min_volume_usd: f64,
    ) -> MarketResult<Option<WalletActivity>> {
        Ok(None)
    }

    async fn ranked_traders(&self, _address: &str) -> Vec<RankedTrader> {
        Vec::new()
    }
}

/// News that remembers what it was asked and has `stories` to tell.
#[derive(Default)]
pub struct FakeNews {
    pub stories: Vec<NewsHit>,
    pub asked: Mutex<Vec<NewsQuery>>,
}

#[async_trait]
impl NewsSearch for FakeNews {
    async fn search_news(&self, query: &NewsQuery) -> Result<Vec<NewsHit>, SearchError> {
        self.asked
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(query.clone());
        Ok(self.stories.clone())
    }
}

/// Eyes that always see the same thing.
pub struct FakeVision(pub String);

#[async_trait]
impl ImageReader for FakeVision {
    async fn describe(
        &self,
        _image: ImageInput,
        _question: Option<&str>,
    ) -> teeny_media::Result<String> {
        Ok(self.0.clone())
    }
}

/// A generator that draws a one-byte PNG and counts its calls.
#[derive(Default)]
pub struct FakeImages {
    pub calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ImageGenerator for FakeImages {
    async fn generate(
        &self,
        prompt: &str,
        _aspect: ImageAspect,
    ) -> teeny_media::Result<GeneratedImage> {
        self.calls
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(prompt.to_owned());
        Ok(GeneratedImage {
            data: vec![1],
            mime_type: MimeType::Png,
        })
    }
}

/// A generator that films a one-byte, four-second clip.
#[derive(Default)]
pub struct FakeVideos;

#[async_trait]
impl VideoGenerator for FakeVideos {
    async fn generate(&self, _request: VideoRequest) -> teeny_media::Result<GeneratedVideo> {
        Ok(GeneratedVideo {
            data: vec![1],
            seconds: Some(4),
        })
    }
}

/// A GIF channel with one GIF in it, `wave`.
#[derive(Default)]
pub struct FakeGifs;

fn wave() -> Gif {
    Gif {
        id: "g1".into(),
        title: "waving hello".into(),
        page_url: "https://giphy.com/gifs/g1".into(),
        media_url: "https://media.giphy.com/g1.gif".into(),
        mime_type: MimeType::Gif,
    }
}

#[async_trait]
impl GifSource for FakeGifs {
    async fn search(&self, query: &str, _channel_only: bool) -> teeny_media::Result<Vec<Gif>> {
        Ok(if query.contains("wave") {
            vec![wave()]
        } else {
            Vec::new()
        })
    }

    async fn get(&self, id: &str) -> teeny_media::Result<Option<Gif>> {
        Ok((id == "g1").then(wave))
    }

    async fn download(&self, gif: &Gif) -> teeny_media::Result<(Vec<u8>, MimeType)> {
        Ok((vec![7], gif.mime_type))
    }
}
