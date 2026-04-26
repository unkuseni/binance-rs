use crate::errors::Result;
use crate::client::Client;
use crate::model::{
    AggrTradesEvent, BookTickerEvent, ContinuousKlineEvent, DayTickerEvent, DepthOrderBookEvent,
    IndexKlineEvent, IndexPriceEvent, KlineEvent, LiquidationEvent, MarkPriceEvent,
    MiniTickerEvent, OrderBook, TradeEvent,
};
use crate::futures::{FuturesMarket, FuturesWebSockets, FuturesWebsocketEvent};
use crate::futures::userstream::FuturesUserStream;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

// ============================================
// New FuturesStream API - Simplified high-level interface for WebSocket subscriptions
// ============================================

/// Stream configuration for futures
#[derive(Clone)]
pub struct FuturesStreamConfig {
    pub market: FuturesMarket,
    pub client: Option<Client>,
    pub recv_window: Option<u64>,
}

impl Default for FuturesStreamConfig {
    fn default() -> Self {
        Self {
            market: FuturesMarket::USDM,
            client: None,
            recv_window: None,
        }
    }
}

/// High-level futures WebSocket stream interface (similar to Bybit's Stream)
#[derive(Clone)]
pub struct FuturesStream {
    config: FuturesStreamConfig,
}

impl Default for FuturesStream {
    fn default() -> Self {
        Self::new()
    }
}

impl FuturesStream {
    /// Create a new FuturesStream with default configuration (public streams only)
    pub fn new() -> Self {
        Self {
            config: FuturesStreamConfig::default(),
        }
    }

    /// Create a new FuturesStream with a client for private streams
    pub fn with_client(client: Client) -> Self {
        Self {
            config: FuturesStreamConfig {
                client: Some(client),
                ..Default::default()
            },
        }
    }

    /// Create a new FuturesStream with custom configuration
    pub fn with_config(config: FuturesStreamConfig) -> Self {
        Self { config }
    }

    /// Subscribe to a single futures stream with a callback handler
    ///
    /// # Arguments
    ///
    /// * `subscription` - Stream name (e.g., "btcusdt@trade")
    /// * `handler` - Callback function to handle events
    pub async fn ws_subscribe<F>(&self, subscription: &str, handler: F) -> Result<()>
    where
        F: FnMut(FuturesWebsocketEvent) -> Result<()> + 'static + Send,
    {
        let mut ws = FuturesWebSockets::new(handler);
        ws.connect(&self.config.market, subscription).await?;
        let running = Arc::new(AtomicBool::new(true));
        ws.event_loop(&running).await
    }

    /// Subscribe to multiple futures streams with a callback handler
    ///
    /// # Arguments
    ///
    /// * `streams` - Vector of stream names (e.g., ["btcusdt@trade", "ethusdt@ticker"])
    /// * `handler` - Callback function to handle events
    pub async fn ws_subscribe_multiple<F>(&self, streams: &[String], handler: F) -> Result<()>
    where
        F: FnMut(FuturesWebsocketEvent) -> Result<()> + 'static + Send,
    {
        let mut ws = FuturesWebSockets::new(handler);
        ws.connect_multiple_streams(&self.config.market, streams)
            .await?;
        let running = Arc::new(AtomicBool::new(true));
        ws.event_loop(&running).await
    }

    /// Subscribe to a single stream with a channel-based output.
    ///
    /// A generic helper that eliminates boilerplate per channel method.
    /// Use this instead of manually calling `ws_subscribe` with a pattern-match closure.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The data type extracted from the event
    /// * `M` - A mapper closure: `FuturesWebsocketEvent -> Option<T>`
    async fn subscribe_channel<T, M>(
        &self, subscription: &str, sender: mpsc::UnboundedSender<T>, mapper: M,
    ) -> Result<()>
    where
        T: Send + 'static,
        M: Fn(FuturesWebsocketEvent) -> Option<T> + Send + 'static,
    {
        self.ws_subscribe(subscription, move |event| {
            if let Some(data) = mapper(event) {
                sender
                    .send(data)
                    .map_err(|e| crate::errors::Error::Msg(format!("Channel send error: {}", e)))?;
            }
            Ok(())
        })
        .await
    }

    /// Subscribe to order book depth updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `depth` - Order book depth (5, 10, 20, 50, 100, 500, 1000, 5000)
    /// * `speed` - Update speed ("100ms" or "1000ms")
    /// * `sender` - Channel sender for order book updates
    pub async fn ws_orderbook(
        &self, symbol: &str, depth: u32, speed: &str, sender: mpsc::UnboundedSender<OrderBook>,
    ) -> Result<()> {
        let subscription = format!("{}@depth{}@{}", symbol.to_lowercase(), depth, speed);
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::OrderBook(book) = event {
                Some(book)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to trade updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for trade updates
    pub async fn ws_trades(
        &self, symbol: &str, sender: mpsc::UnboundedSender<TradeEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@trade", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::Trade(trade) = event {
                Some(trade)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to kline/candlestick updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `interval` - Kline interval (e.g., "1m", "5m", "1h", "1d")
    /// * `sender` - Channel sender for kline updates
    pub async fn ws_klines(
        &self, symbol: &str, interval: &str, sender: mpsc::UnboundedSender<KlineEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@kline_{}", symbol.to_lowercase(), interval);
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::Kline(kline) = event {
                Some(kline)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for ticker updates
    pub async fn ws_ticker(
        &self, symbol: &str, sender: mpsc::UnboundedSender<DayTickerEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@ticker", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::DayTicker(ticker) = event {
                Some(ticker)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to mini ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for mini ticker updates
    pub async fn ws_mini_ticker(
        &self, symbol: &str, sender: mpsc::UnboundedSender<MiniTickerEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@miniTicker", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::MiniTicker(ticker) = event {
                Some(ticker)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to book ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for book ticker updates
    pub async fn ws_book_ticker(
        &self, symbol: &str, sender: mpsc::UnboundedSender<BookTickerEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@bookTicker", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::BookTicker(ticker) = event {
                Some(ticker)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to mark price updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for mark price updates
    pub async fn ws_mark_price(
        &self, symbol: &str, sender: mpsc::UnboundedSender<MarkPriceEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@markPrice", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::MarkPrice(mark_price) = event {
                Some(mark_price)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to liquidation updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for liquidation updates
    pub async fn ws_liquidations(
        &self, symbol: &str, sender: mpsc::UnboundedSender<LiquidationEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@forceOrder", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::Liquidation(liquidation) = event {
                Some(liquidation)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to aggregate trades updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for aggregate trade updates
    pub async fn ws_agg_trades(
        &self, symbol: &str, sender: mpsc::UnboundedSender<AggrTradesEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@aggTrade", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::AggrTrades(agg_trade) = event {
                Some(agg_trade)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to index price updates with channel output
    ///
    /// # Arguments
    ///
    /// * `pair` - Trading pair (e.g., "BTCUSD" for index)
    /// * `sender` - Channel sender for index price updates
    pub async fn ws_index_price(
        &self, pair: &str, sender: mpsc::UnboundedSender<IndexPriceEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@indexPrice", pair.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::IndexPrice(index_price) = event {
                Some(index_price)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to continuous kline updates with channel output
    ///
    /// # Arguments
    ///
    /// * `pair` - Trading pair (e.g., "BTCUSD")
    /// * `contract_type` - Contract type ("perpetual", "current_quarter", "next_quarter")
    /// * `interval` - Kline interval (e.g., "1m", "5m", "1h", "1d")
    /// * `sender` - Channel sender for continuous kline updates
    pub async fn ws_continuous_klines(
        &self, pair: &str, contract_type: &str, interval: &str,
        sender: mpsc::UnboundedSender<ContinuousKlineEvent>,
    ) -> Result<()> {
        let subscription = format!(
            "{}_{}@continuousKline_{}",
            pair.to_lowercase(),
            contract_type,
            interval
        );
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::ContinuousKline(continuous_kline) = event {
                Some(continuous_kline)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to index kline updates with channel output
    ///
    /// # Arguments
    ///
    /// * `pair` - Trading pair (e.g., "BTCUSD")
    /// * `interval` - Kline interval (e.g., "1m", "5m", "1h", "1d")
    /// * `sender` - Channel sender for index kline updates
    pub async fn ws_index_klines(
        &self, pair: &str, interval: &str, sender: mpsc::UnboundedSender<IndexKlineEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@indexPriceKline_{}", pair.to_lowercase(), interval);
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::IndexKline(index_kline) = event {
                Some(index_kline)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to depth order book updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for depth order book updates
    pub async fn ws_depth_orderbook(
        &self, symbol: &str, sender: mpsc::UnboundedSender<DepthOrderBookEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@depth", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::DepthOrderBook(depth_orderbook) = event {
                Some(depth_orderbook)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to depth order book updates with specific levels and speed
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `levels` - Order book levels (5, 10, 20, 50, 100, 500, 1000, 5000)
    /// * `speed` - Update speed ("100ms" or "1000ms")
    /// * `sender` - Channel sender for depth order book updates
    pub async fn ws_depth_orderbook_levels(
        &self, symbol: &str, levels: u32, speed: &str,
        sender: mpsc::UnboundedSender<DepthOrderBookEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@depth{}@{}", symbol.to_lowercase(), levels, speed);
        self.subscribe_channel(&subscription, sender, |event| {
            if let FuturesWebsocketEvent::DepthOrderBook(depth_orderbook) = event {
                Some(depth_orderbook)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to all mini ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for all mini ticker updates (Vec)
    pub async fn ws_all_mini_tickers(
        &self, sender: mpsc::UnboundedSender<Vec<MiniTickerEvent>>,
    ) -> Result<()> {
        let subscription = "!miniTicker@arr";
        self.subscribe_channel(subscription, sender, |event| {
            if let FuturesWebsocketEvent::MiniTickerAll(mini_tickers) = event {
                Some(mini_tickers)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to all ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for all ticker updates (Vec)
    pub async fn ws_all_tickers(
        &self, sender: mpsc::UnboundedSender<Vec<DayTickerEvent>>,
    ) -> Result<()> {
        let subscription = "!ticker@arr";
        self.subscribe_channel(subscription, sender, |event| {
            if let FuturesWebsocketEvent::DayTickerAll(tickers) = event {
                Some(tickers)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to all book ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for all book ticker updates
    pub async fn ws_all_book_tickers(
        &self, sender: mpsc::UnboundedSender<BookTickerEvent>,
    ) -> Result<()> {
        let subscription = "!bookTicker";
        self.subscribe_channel(subscription, sender, |event| {
            if let FuturesWebsocketEvent::BookTicker(book_ticker) = event {
                Some(book_ticker)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to all liquidation updates with channel output
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for all liquidation updates
    pub async fn ws_all_liquidations(
        &self, sender: mpsc::UnboundedSender<LiquidationEvent>,
    ) -> Result<()> {
        let subscription = "!forceOrder@arr";
        self.subscribe_channel(subscription, sender, |event| {
            if let FuturesWebsocketEvent::Liquidation(liquidation) = event {
                Some(liquidation)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to all mark price updates with channel output
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for all mark price updates (Vec)
    pub async fn ws_all_mark_prices(
        &self, sender: mpsc::UnboundedSender<Vec<MarkPriceEvent>>,
    ) -> Result<()> {
        let subscription = "!markPrice@arr";
        self.subscribe_channel(subscription, sender, |event| {
            if let FuturesWebsocketEvent::MarkPriceAll(mark_prices) = event {
                Some(mark_prices)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to user data stream (private)
    ///
    /// # Arguments
    ///
    /// * `listen_key` - User data stream listen key
    /// * `sender` - Channel sender for user data events
    ///
    /// # Note
    ///
    /// Requires a client with API key and secret
    pub async fn ws_user_data(
        &self, listen_key: &str, sender: mpsc::UnboundedSender<FuturesWebsocketEvent>,
    ) -> Result<()> {
        let subscription = listen_key;
        self.subscribe_channel(subscription, sender, Some).await
    }

    /// Subscribe to user data stream with automatic listen key management
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for user data events
    ///
    /// # Note
    ///
    /// Requires a client with API key and secret
    pub async fn ws_user_data_auto(
        &self, sender: mpsc::UnboundedSender<FuturesWebsocketEvent>,
    ) -> Result<()> {
        let client = self.config.client.as_ref().ok_or_else(|| {
            crate::errors::Error::Msg("Client required for user data stream".to_string())
        })?;

        // Start user data stream
        let user_stream = FuturesUserStream {
            client: client.clone(),
            recv_window: self.config.recv_window.unwrap_or(5000),
        };

        let listen_key_data = user_stream.start().await?;
        let listen_key = listen_key_data.listen_key;

        // Spawn keep-alive task
        let keep_alive_client = client.clone();
        let recv_window = self.config.recv_window.unwrap_or(5000);
        let listen_key_clone = listen_key.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30 * 60)); // 30 minutes
            let user_stream = FuturesUserStream {
                client: keep_alive_client,
                recv_window,
            };

            loop {
                interval.tick().await;
                if let Err(e) = user_stream.keep_alive(&listen_key_clone).await {
                    eprintln!("Failed to keep alive user stream: {}", e);
                }
            }
        });

        self.ws_user_data(&listen_key, sender).await
    }
}
