use crate::errors::Result;
use crate::config::Config;
use crate::model::{
    AccountUpdateEvent, AggrTradesEvent, BookTickerEvent, ContinuousKlineEvent, DayTickerEvent,
    DepthOrderBookEvent, IndexKlineEvent, IndexPriceEvent, KlineEvent, LiquidationEvent,
    MarkPriceEvent, MiniTickerEvent, OrderBook, TradeEvent, UserDataStreamExpiredEvent,
};
use crate::futures::model;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_tungstenite::tungstenite::Message;
use futures_util::{SinkExt, StreamExt};

enum FuturesWebsocketAPI {
    Default,
    MultiStream,
    Custom(String),
}

#[derive(Debug, Clone, Copy)]
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

        // Determine the routing path based on stream type
        let path = if subscription.contains("bookTicker") || subscription.contains("depth") {
            "/public"
        } else {
            "/market"
        };

        match self {
            FuturesWebsocketAPI::Default => {
                format!("{}{}/ws/{}", baseurl, path, subscription)
            }
            FuturesWebsocketAPI::MultiStream => {
                format!("{}{}/stream?streams={}", baseurl, path, subscription)
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
    pub(crate) ws_client: Option<crate::ws_core::WsClient>,
    handler: Box<dyn FnMut(FuturesWebsocketEvent) -> Result<()> + 'a + Send>,
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
        FuturesWebSockets {
            ws_client: None,
            handler: Box::new(handler),
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
        self.ws_client = Some(crate::ws_core::WsClient::connect(wss).await?);
        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut client) = self.ws_client.take() {
            client.disconnect().await?;
            Ok(())
        } else {
            Err(crate::errors::Error::Msg(
                "Not able to close the connection".to_string(),
            ))
        }
    }

    pub fn test_handle_msg(&mut self, msg: &str) -> Result<()> {
        self.handle_msg(msg)
    }

    pub fn handle_msg(&mut self, msg: &str) -> Result<()> {
        self.handle_msg_depth(msg, 0)
    }

    fn handle_msg_depth(&mut self, msg: &str, depth: u32) -> Result<()> {
        if depth > 5 {
            return Ok(());
        }
        let value: serde_json::Value = serde_json::from_str(msg)?;

        if let Some(data) = value.get("data") {
            self.handle_msg_depth(&data.to_string(), depth + 1)?;
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
        while running.load(Ordering::Acquire) {
            if let Some(ref mut socket) = self.ws_client.as_mut().and_then(|c| c.socket.as_mut()) {
                match socket.next().await {
                    Some(Ok(message)) => match message {
                        Message::Text(msg) => {
                            if let Err(e) = self.handle_msg(&msg) {
                                return Err(crate::errors::Error::Msg(format!(
                                    "Error on handling stream message: {}",
                                    e
                                )));
                            }
                        }
                        Message::Ping(data) => {
                            let _ = socket.send(Message::Pong(data)).await;
                        }
                        Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => (),
                        Message::Close(e) => {
                            return Err(crate::errors::Error::Msg(format!("Disconnected {:?}", e)))
                        }
                    },
                    Some(Err(e)) => {
                        return Err(crate::errors::Error::Msg(format!("WebSocket error: {}", e)))
                    }
                    None => {
                        return Err(crate::errors::Error::Msg(
                            "WebSocket stream ended".to_string(),
                        ))
                    }
                }
            } else {
                return Err(crate::errors::Error::Msg(
                    "WebSocket is not connected".to_string(),
                ));
            }
        }
        Ok(())
    }
}
