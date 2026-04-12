use crate::errors::Result;
use crate::config::Config;
use crate::model::{
    AccountUpdateEvent, AggrTradesEvent, BookTickerEvent, ContinuousKlineEvent, DayTickerEvent,
    DepthOrderBookEvent, IndexKlineEvent, IndexPriceEvent, KlineEvent, LiquidationEvent,
    MarkPriceEvent, MiniTickerEvent, OrderBook, TradeEvent, UserDataStreamExpiredEvent,
};
use crate::futures::model;
use url::Url;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures_util::{SinkExt, StreamExt};

enum FuturesWebsocketAPI {
    Default,
    MultiStream,
    Custom(String),
}

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
    handler: Box<dyn FnMut(FuturesWebsocketEvent) -> Result<()> + 'a>,
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
        Callback: FnMut(FuturesWebsocketEvent) -> Result<()> + 'a,
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
