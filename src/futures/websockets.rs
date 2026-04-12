use crate::errors::Result;
use crate::config::Config;
use crate::client::Client;
use crate::model::{
    AccountUpdateEvent, AggrTradesEvent, BookTickerEvent, ContinuousKlineEvent, DayTickerEvent,
    DepthOrderBookEvent, IndexKlineEvent, IndexPriceEvent, KlineEvent, LiquidationEvent,
    MarkPriceEvent, MiniTickerEvent, OrderBook, TradeEvent, UserDataStreamExpiredEvent,
};
use crate::futures::model;
use crate::futures::userstream::FuturesUserStream;
use url::Url;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures_util::{SinkExt, StreamExt};

const PING_INTERVAL_SECONDS: u64 = 30;

// ============================================
// New FuturesStream API (Bybit-style)
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::OrderBook(book) = event {
                sender.send(book).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::Trade(trade) = event {
                sender.send(trade).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::Kline(kline) = event {
                sender.send(kline).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::DayTicker(ticker) = event {
                sender.send(ticker).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::MiniTicker(ticker) = event {
                sender.send(ticker).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::BookTicker(ticker) = event {
                sender.send(ticker).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::MarkPrice(mark_price) = event {
                sender.send(mark_price).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::Liquidation(liquidation) = event {
                sender.send(liquidation).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::AggrTrades(agg_trade) = event {
                sender.send(agg_trade).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::IndexPrice(index_price) = event {
                sender.send(index_price).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::ContinuousKline(continuous_kline) = event {
                sender.send(continuous_kline).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::IndexKline(index_kline) = event {
                sender.send(index_kline).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::DepthOrderBook(depth_orderbook) = event {
                sender.send(depth_orderbook).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let FuturesWebsocketEvent::DepthOrderBook(depth_orderbook) = event {
                sender.send(depth_orderbook).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(subscription, move |event| {
            if let FuturesWebsocketEvent::MiniTickerAll(mini_tickers) = event {
                sender.send(mini_tickers).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(subscription, move |event| {
            if let FuturesWebsocketEvent::DayTickerAll(tickers) = event {
                sender.send(tickers).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(subscription, move |event| {
            if let FuturesWebsocketEvent::BookTicker(book_ticker) = event {
                sender.send(book_ticker).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(subscription, move |event| {
            if let FuturesWebsocketEvent::Liquidation(liquidation) = event {
                sender.send(liquidation).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(subscription, move |event| {
            if let FuturesWebsocketEvent::MarkPriceAll(mark_prices) = event {
                sender.send(mark_prices).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(subscription, move |event| {
            sender.send(event).map_err(|e| {
                crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                    "Channel send error: {}",
                    e
                )))
            })?;
            Ok(())
        })
        .await
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
            crate::errors::ErrorKind::Msg("Client required for user data stream".to_string())
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

    /// Connect to futures WebSocket with dynamic command control
    ///
    /// # Arguments
    ///
    /// * `subscription` - Stream name
    /// * `cmd_receiver` - Channel receiver for commands
    /// * `handler` - Callback function to handle events
    pub async fn ws_subscribe_with_commands<'a, F>(
        &self, subscription: &str, cmd_receiver: mpsc::UnboundedReceiver<FuturesWebsocketCommand>,
        handler: F,
    ) -> Result<()>
    where
        F: FnMut(FuturesWebsocketEvent) -> Result<()> + 'static + Send,
    {
        let mut ws = FuturesWebSockets::new(handler);
        ws.connect(&self.config.market, subscription).await?;

        // Transfer command receiver to FuturesWebSockets
        if let Some(ws_cmd_tx) = ws.get_command_sender() {
            tokio::spawn(async move {
                let mut cmd_rx = cmd_receiver;
                while let Some(cmd) = cmd_rx.recv().await {
                    let _ = ws_cmd_tx.send(cmd);
                }
            });
        }

        let running = Arc::new(AtomicBool::new(true));
        ws.event_loop(&running).await
    }
}

// ============================================
// Legacy FuturesWebSockets API (maintained for compatibility)
// ============================================

enum FuturesWebsocketAPI {
    Default,
    MultiStream,
    Custom(String),
}

#[derive(Clone)]
pub enum FuturesMarket {
    USDM,
    COINM,
    Vanilla,
    USDMTestnet,
    COINMTestnet,
    VanillaTestnet,
}

impl FuturesWebsocketAPI {
    fn params(self, market: &FuturesMarket, subscription: &str) -> String {
        let baseurl = match market {
            FuturesMarket::USDM => "wss://fstream.binance.com",
            FuturesMarket::COINM => "wss://dstream.binance.com",
            FuturesMarket::Vanilla => "wss://vstream.binance.com",
            FuturesMarket::USDMTestnet => "wss://fstream.binancefuture.com",
            FuturesMarket::COINMTestnet => "wss://dstream.binancefuture.com",
            FuturesMarket::VanillaTestnet => "wss://vstream.binancefuture.com",
        };

        match self {
            FuturesWebsocketAPI::Default => {
                format!("{}/ws/{}", baseurl, subscription)
            }
            FuturesWebsocketAPI::MultiStream => {
                format!("{}/stream?streams={}", baseurl, subscription)
            }
            FuturesWebsocketAPI::Custom(url) => url,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum FuturesWebsocketEvent {
    AccountUpdate(AccountUpdateEvent),
    OrderTrade(model::OrderTradeEvent),
    AggrTrades(AggrTradesEvent),
    Trade(TradeEvent),
    OrderBook(OrderBook),
    DayTicker(DayTickerEvent),
    MiniTicker(MiniTickerEvent),
    MiniTickerAll(Vec<MiniTickerEvent>),
    IndexPrice(IndexPriceEvent),
    MarkPrice(MarkPriceEvent),
    MarkPriceAll(Vec<MarkPriceEvent>),
    DayTickerAll(Vec<DayTickerEvent>),
    Kline(KlineEvent),
    ContinuousKline(ContinuousKlineEvent),
    IndexKline(IndexKlineEvent),
    Liquidation(LiquidationEvent),
    DepthOrderBook(DepthOrderBookEvent),
    BookTicker(BookTickerEvent),
    UserDataStreamExpiredEvent(UserDataStreamExpiredEvent),
}

pub struct FuturesWebSockets<'a> {
    pub socket: Option<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    handler: Box<dyn FnMut(FuturesWebsocketEvent) -> Result<()> + Send + 'a>,
    command_tx: Option<mpsc::UnboundedSender<FuturesWebsocketCommand>>,
    command_rx: Option<mpsc::UnboundedReceiver<FuturesWebsocketCommand>>,
}

#[derive(Debug)]
pub enum FuturesWebsocketCommand {
    Subscribe(String),
    Unsubscribe(String),
    Disconnect,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum FuturesEvents {
    Vec(Vec<DayTickerEvent>),
    DayTickerEvent(DayTickerEvent),
    BookTickerEvent(BookTickerEvent),
    MiniTickerEvent(MiniTickerEvent),
    VecMiniTickerEvent(Vec<MiniTickerEvent>),
    AccountUpdateEvent(AccountUpdateEvent),
    OrderTradeEvent(model::OrderTradeEvent),
    AggrTradesEvent(AggrTradesEvent),
    IndexPriceEvent(IndexPriceEvent),
    MarkPriceEvent(MarkPriceEvent),
    VecMarkPriceEvent(Vec<MarkPriceEvent>),
    TradeEvent(TradeEvent),
    KlineEvent(KlineEvent),
    ContinuousKlineEvent(ContinuousKlineEvent),
    IndexKlineEvent(IndexKlineEvent),
    LiquidationEvent(LiquidationEvent),
    OrderBook(OrderBook),
    DepthOrderBookEvent(DepthOrderBookEvent),
    UserDataStreamExpiredEvent(UserDataStreamExpiredEvent),
}

impl<'a> FuturesWebSockets<'a> {
    pub fn new<Callback>(handler: Callback) -> FuturesWebSockets<'a>
    where
        Callback: FnMut(FuturesWebsocketEvent) -> Result<()> + 'a + Send,
    {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        FuturesWebSockets {
            socket: None,
            handler: Box::new(handler),
            command_tx: Some(command_tx),
            command_rx: Some(command_rx),
        }
    }

    pub async fn connect(&mut self, market: &FuturesMarket, subscription: &str) -> Result<()> {
        self.connect_wss(&FuturesWebsocketAPI::Default.params(market, subscription))
            .await
    }

    pub async fn connect_with_config(
        &mut self, market: &FuturesMarket, subscription: &str, config: &Config,
    ) -> Result<()> {
        self.connect_wss(
            &FuturesWebsocketAPI::Custom(config.ws_endpoint.clone()).params(market, subscription),
        )
        .await
    }

    pub async fn connect_multiple_streams(
        &mut self, market: &FuturesMarket, endpoints: &[String],
    ) -> Result<()> {
        self.connect_wss(&FuturesWebsocketAPI::MultiStream.params(market, &endpoints.join("/")))
            .await
    }

    async fn connect_wss(&mut self, wss: &str) -> Result<()> {
        let url = Url::parse(wss)?;
        match connect_async(url).await {
            Ok((socket, _)) => {
                self.socket = Some(socket);
                Ok(())
            }
            Err(e) => Err(format!("Error during handshake {}", e).into()),
        }
    }

    pub fn disconnect(&mut self) -> Result<()> {
        if let Some(_) = self.socket {
            // Note: We need to close the socket properly
            // In async context, we should use async close
            // For now, we'll just drop the socket
            self.socket = None;
            return Ok(());
        }
        Err("Not able to close the connection".into())
    }

    pub fn test_handle_msg(&mut self, msg: &str) -> Result<()> {
        self.handle_msg(msg)
    }

    pub fn handle_msg(&mut self, msg: &str) -> Result<()> {
        let value: serde_json::Value = serde_json::from_str(msg)?;

        if let Some(data) = value.get("data") {
            self.handle_msg(&data.to_string())?;
            return Ok(());
        }

        if let Ok(events) = serde_json::from_value::<FuturesEvents>(value) {
            let action = match events {
                FuturesEvents::Vec(v) => FuturesWebsocketEvent::DayTickerAll(v),
                FuturesEvents::DayTickerEvent(v) => FuturesWebsocketEvent::DayTicker(v),
                FuturesEvents::BookTickerEvent(v) => FuturesWebsocketEvent::BookTicker(v),
                FuturesEvents::MiniTickerEvent(v) => FuturesWebsocketEvent::MiniTicker(v),
                FuturesEvents::VecMiniTickerEvent(v) => FuturesWebsocketEvent::MiniTickerAll(v),
                FuturesEvents::AccountUpdateEvent(v) => FuturesWebsocketEvent::AccountUpdate(v),
                FuturesEvents::OrderTradeEvent(v) => FuturesWebsocketEvent::OrderTrade(v),
                FuturesEvents::IndexPriceEvent(v) => FuturesWebsocketEvent::IndexPrice(v),
                FuturesEvents::MarkPriceEvent(v) => FuturesWebsocketEvent::MarkPrice(v),
                FuturesEvents::VecMarkPriceEvent(v) => FuturesWebsocketEvent::MarkPriceAll(v),
                FuturesEvents::TradeEvent(v) => FuturesWebsocketEvent::Trade(v),
                FuturesEvents::ContinuousKlineEvent(v) => FuturesWebsocketEvent::ContinuousKline(v),
                FuturesEvents::IndexKlineEvent(v) => FuturesWebsocketEvent::IndexKline(v),
                FuturesEvents::LiquidationEvent(v) => FuturesWebsocketEvent::Liquidation(v),
                FuturesEvents::KlineEvent(v) => FuturesWebsocketEvent::Kline(v),
                FuturesEvents::OrderBook(v) => FuturesWebsocketEvent::OrderBook(v),
                FuturesEvents::DepthOrderBookEvent(v) => FuturesWebsocketEvent::DepthOrderBook(v),
                FuturesEvents::AggrTradesEvent(v) => FuturesWebsocketEvent::AggrTrades(v),
                FuturesEvents::UserDataStreamExpiredEvent(v) => {
                    FuturesWebsocketEvent::UserDataStreamExpiredEvent(v)
                }
            };
            (self.handler)(action)?;
        }
        Ok(())
    }

    pub async fn event_loop(&mut self, running: &AtomicBool) -> Result<()> {
        self.event_loop_with_cancellation(Some(running), None).await
    }

    pub async fn event_loop_with_cancellation(
        &mut self, running: Option<&AtomicBool>, mut cancel_rx: Option<mpsc::UnboundedReceiver<()>>,
    ) -> Result<()> {
        let mut command_rx = self.command_rx.take().expect("command_rx already taken");
        let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECONDS));
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if let Some(running) = running {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
            }

            tokio::select! {
                // Handle WebSocket messages
                message = async {
                    if let Some(ref mut socket) = self.socket {
                        socket.next().await
                    } else {
                        None
                    }
                } => {
                    match message {
                        Some(Ok(message)) => match message {
                            Message::Text(msg) => {
                                if let Err(e) = self.handle_msg(&msg) {
                                    return Err(
                                        format!("Error on handling stream message: {}", e).into()
                                    );
                                }
                            }
                            Message::Ping(data) => {
                                if let Some(ref mut socket) = self.socket {
                                    let _ = socket.send(Message::Pong(data)).await;
                                }
                            }
                            Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => (),
                            Message::Close(e) => return Err(format!("Disconnected {:?}", e).into()),
                        },
                        Some(Err(e)) => return Err(format!("WebSocket error: {}", e).into()),
                        None => return Err("WebSocket stream ended".into()),
                    }
                }

                // Handle commands from command channel
                command = command_rx.recv() => {
                    match command {
                        Some(FuturesWebsocketCommand::Disconnect) => {
                            if let Some(ref mut socket) = self.socket {
                                let _ = socket.close(None).await;
                            }
                            self.socket = None;
                            break;
                        }
                        Some(FuturesWebsocketCommand::Subscribe(_stream)) => {
                            // Note: Binance WebSocket doesn't support dynamic subscription
                            // This is a placeholder for future implementation
                        }
                        Some(FuturesWebsocketCommand::Unsubscribe(_stream)) => {
                            // Note: Binance WebSocket doesn't support dynamic unsubscription
                            // This is a placeholder for future implementation
                        }
                        None => {
                            // Command channel closed
                            break;
                        }
                    }
                }

                // Handle cancellation signal
                _ = async {
                    if let Some(ref mut cancel_rx) = cancel_rx {
                        cancel_rx.recv().await
                    } else {
                        None
                    }
                } => {
                    // Cancellation signal received
                    break;
                }

                // Send periodic ping
                _ = ping_interval.tick() => {
                    if let Some(ref mut socket) = self.socket {
                        let _ = socket.send(Message::Ping(vec![])).await;
                    }
                }
            }
        }

        self.command_rx = Some(command_rx);
        Ok(())
    }

    pub fn get_command_sender(&self) -> Option<mpsc::UnboundedSender<FuturesWebsocketCommand>> {
        self.command_tx.clone()
    }

    pub fn subscribe(&self, stream: String) -> Result<()> {
        if let Some(ref tx) = self.command_tx {
            tx.send(FuturesWebsocketCommand::Subscribe(stream))
                .map_err(|e| format!("Failed to send subscribe command: {}", e).into())
        } else {
            Err("Command channel not available".into())
        }
    }

    pub fn unsubscribe(&self, stream: String) -> Result<()> {
        if let Some(ref tx) = self.command_tx {
            tx.send(FuturesWebsocketCommand::Unsubscribe(stream))
                .map_err(|e| format!("Failed to send unsubscribe command: {}", e).into())
        } else {
            Err("Command channel not available".into())
        }
    }

    pub fn disconnect_via_command(&self) -> Result<()> {
        if let Some(ref tx) = self.command_tx {
            tx.send(FuturesWebsocketCommand::Disconnect)
                .map_err(|e| format!("Failed to send disconnect command: {}", e).into())
        } else {
            Err("Command channel not available".into())
        }
    }
}
