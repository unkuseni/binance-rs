#![allow(dead_code)]

use binance::api::*;
use binance::userstream::*;
use binance::websockets_old::*;
use tokio::time;
use tokio::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

// Note: This example demonstrates the traditional WebSockets API.
// For a more modern, channel-based API, see the `Stream` struct in the websockets module
// and examples/binance_websockets_modern.rs

#[tokio::main]
async fn main() {
    market_websocket().await;
}

async fn user_stream() {
    let api_key_user = Some("YOUR_API_KEY".into());
    let user_stream: UserStream = Binance::new(api_key_user, None);

    if let Ok(answer) = user_stream.start().await {
        println!("Data Stream Started ...");
        let listen_key = answer.listen_key;

        match user_stream.keep_alive(&listen_key).await {
            Ok(msg) => println!("Keepalive user data stream: {:?}", msg),
            Err(e) => println!("Error: {}", e),
        }

        match user_stream.close(&listen_key).await {
            Ok(msg) => println!("Close user data stream: {:?}", msg),
            Err(e) => println!("Error: {}", e),
        }
    } else {
        println!("Not able to start an User Stream (Check your API_KEY)");
    }
}

async fn user_stream_websocket() {
    let keep_running = AtomicBool::new(true); // Used to control the event loop
    let api_key_user = Some("YOUR_KEY".into());
    let user_stream: UserStream = Binance::new(api_key_user, None);

    if let Ok(answer) = user_stream.start().await {
        let listen_key = answer.listen_key;

        let mut web_socket: WebSockets<'_> = WebSockets::new(|event: WebsocketEvent| {
            match event {
                WebsocketEvent::AccountUpdate(account_update) => {
                    for balance in &account_update.data.balances {
                        println!(
                            "Asset: {}, wallet_balance: {}, cross_wallet_balance: {}, balance: {}",
                            balance.asset,
                            balance.wallet_balance,
                            balance.cross_wallet_balance,
                            balance.balance_change
                        );
                    }
                }
                WebsocketEvent::OrderTrade(trade) => {
                    println!(
                        "Symbol: {}, Side: {}, Price: {}, Execution Type: {}",
                        trade.symbol, trade.side, trade.price, trade.execution_type
                    );
                }
                _ => (),
            };

            Ok(())
        });

        web_socket.connect(&listen_key).await.unwrap(); // check error
        if let Err(e) = web_socket.event_loop(&keep_running).await {
            println!("Error: {}", e);
        }
        user_stream.close(&listen_key).await.unwrap();
        web_socket.disconnect().unwrap();
        println!("Userstrem closed and disconnected");
    } else {
        println!("Not able to start an User Stream (Check your API_KEY)");
    }
}

async fn market_websocket() {
    // Keep the loop running so we continuously print updates
    // Use an Arc so we can stop the loop from a spawned task (timeout)
    let keep_running =AtomicBool::new(true); // Used to control the event loop

    // Listen specifically for BTCUSDT depth updates (high-frequency)
    // Options:
    //  - "btcusdt@depth@100ms"    : depth updates every 100ms
    //  - "btcusdt@depth20@100ms"  : depth top20 every 100ms
    let depth_stream = String::from("btcusdt@depth@100ms");

    let mut web_socket: WebSockets<'_> = WebSockets::new(|event: WebsocketEvent| {
        // Only handle depth updates here and print them continuously
        if let WebsocketEvent::DepthOrderBook(depth) = event {
            // Print a concise summary of the update: first few bids/asks and update id
            let mut bids_preview: Vec<_> = depth.bids.iter().take(5).cloned().collect();
            let asks_preview: Vec<_> = depth.asks.iter().take(5).cloned().collect();
            println!(
                "[BTCUSDT DEPTH] last_update_id: {:#?}, bids (top {:#?}): {:#?}, asks (top {:#?}): {:#?}",
                depth.final_update_id,
                bids_preview.len(),
                bids_preview,
                asks_preview.len(),
                asks_preview
            );
        }
        Ok(())
    });

    println!("Connecting to {}...", depth_stream);
    web_socket.connect(&depth_stream).await.unwrap(); // check error

    println!("Continuously printing BTCUSDT depth updates...");
    // This will run until you stop the process (Ctrl+C) or disconnect manually

    if let Err(e) = web_socket.event_loop(&keep_running).await {
        println!("Error: {}", e);
    }

    // If we ever reach here, attempt to cleanly disconnect
    // let _ = web_socket.disconnect();
    // println!("Disconnected");
}

async fn all_trades_websocket() {
    let keep_running = AtomicBool::new(true); // Used to control the event loop
    let agg_trade = String::from("!ticker@arr");
    let mut web_socket = WebSockets::new(|event: WebsocketEvent| {
        if let WebsocketEvent::DayTickerAll(ticker_events) = event {
            for tick_event in ticker_events {
                println!(
                    "Symbol: {}, price: {}, qty: {}",
                    tick_event.symbol, tick_event.best_bid, tick_event.best_bid_qty
                );
            }
        }

        Ok(())
    });

    web_socket.connect(&agg_trade).await.unwrap(); // check error
    if let Err(e) = web_socket.event_loop(&keep_running).await {
        println!("Error: {}", e);
    }
    web_socket.disconnect().unwrap();
    println!("disconnected");
}

async fn kline_websocket() {
    let keep_running = AtomicBool::new(true);
    let kline = String::from("ethbtc@kline_1m");
    let mut web_socket = WebSockets::new(|event: WebsocketEvent| {
        if let WebsocketEvent::Kline(kline_event) = event {
            println!(
                "Symbol: {}, high: {}, low: {}",
                kline_event.kline.symbol, kline_event.kline.low, kline_event.kline.high
            );
        }

        Ok(())
    });

    web_socket.connect(&kline).await.unwrap(); // check error
    if let Err(e) = web_socket.event_loop(&keep_running).await {
        println!("Error: {}", e);
    }
    web_socket.disconnect().unwrap();
    println!("disconnected");
}

async fn last_price_for_one_symbol() {
    let keep_running = AtomicBool::new(true);
    let agg_trade = String::from("btcusdt@ticker");
    let mut btcusdt: f32 = "0".parse().unwrap();

    let mut web_socket = WebSockets::new(|event: WebsocketEvent| {
        if let WebsocketEvent::DayTicker(ticker_event) = event {
            btcusdt = ticker_event.average_price.parse().unwrap();
            let btcusdt_close: f32 = ticker_event.current_close.parse().unwrap();
            println!("{} - {}", btcusdt, btcusdt_close);

            if btcusdt_close as i32 == 7000 {
                // Break the event loop
                keep_running.store(false, Ordering::Relaxed);
            }
        }

        Ok(())
    });

    web_socket.connect(&agg_trade).await.unwrap(); // check error
    if let Err(e) = web_socket.event_loop(&keep_running).await {
        println!("Error: {}", e);
    }
    web_socket.disconnect().unwrap();
    println!("disconnected");
}

async fn multiple_streams() {
    let endpoints =
        ["ETHBTC", "BNBETH"].map(|symbol| format!("{}@depth@100ms", symbol.to_lowercase()));

    let keep_running = AtomicBool::new(true);
    let mut web_socket = WebSockets::new(|event: WebsocketEvent| {
        if let WebsocketEvent::DepthOrderBook(depth_order_book) = event {
            println!("{:?}", depth_order_book);
        }

        Ok(())
    });

    web_socket
        .connect_multiple_streams(&endpoints)
        .await
        .unwrap(); // check error
    if let Err(e) = web_socket.event_loop(&keep_running).await {
        println!("Error: {}", e);
    }
    time::sleep(Duration::from_secs(60)).await;
    web_socket.disconnect().unwrap();
}
