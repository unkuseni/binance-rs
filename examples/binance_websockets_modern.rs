//! Modern WebSocket API example for Binance
//!
//! This example demonstrates the new Stream API implementation with:
//! 1. Channel-based output instead of callbacks
//! 2. Simplified subscription methods
//! 3. Automatic keep-alive for user streams
//! 4. Better error handling and type safety
//! 5. Comparison with legacy WebSockets API

use binance::{
    websockets::{Market, Stream},
    model::{TradeEvent, DayTickerEvent, DepthOrderBookEvent, KlineEvent},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    println!("=== Modern Binance WebSocket Example ===\n");

    // Example 1: New Stream API - Trades
    println!("1. New Stream API - Trades stream:");
    example_stream_trades().await;

    // Example 2: New Stream API - Orderbook
    println!("\n2. New Stream API - Orderbook stream:");
    example_stream_orderbook().await;

    // Example 3: New Stream API - Klines
    println!("\n3. New Stream API - Kline stream:");
    example_stream_klines().await;

    // Example 4: New Stream API - Ticker
    println!("\n4. New Stream API - Ticker stream:");
    example_stream_ticker().await;

    // Example 5: New Stream API - User data with auto keep-alive
    println!("\n5. New Stream API - User data stream (with auto keep-alive):");
    example_stream_user_data_auto().await;

    // Example 6: Multiple streams with new API
    println!("\n6. New Stream API - Multiple streams:");
    example_stream_multiple().await;

    // Example 7: Comparison with legacy WebSockets API
    println!("\n7. Legacy WebSockets API (for comparison):");
    example_legacy_websockets().await;

    println!("\n=== All examples completed ===");
}

/// Example 1: Using the new Stream API for trades
async fn example_stream_trades() {
    let stream = Stream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<TradeEvent>();

    println!("Subscribing to BTCUSDT trades...");

    // Run stream and event processing concurrently in the same task
    let stream_future = stream.ws_trades("BTCUSDT", tx);

    tokio::select! {
        result = stream_future => {
            if let Err(e) = result {
                eprintln!("Stream error: {}", e);
            }
        }
        _ = async {
            let mut count = 0;
            while let Some(trade) = rx.recv().await {
                count += 1;
                println!(
                    "Trade #{}: {} @ {} (qty: {})",
                    count, trade.symbol, trade.price, trade.qty
                );
                if count >= 5 {
                    break;
                }
            }
            println!("Received {} trades", count);
        } => {}
    }

    println!("Trades stream completed");
}

/// Example 2: Using the new Stream API for orderbook
async fn example_stream_orderbook() {
    let stream = Stream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<DepthOrderBookEvent>();

    println!("Subscribing to BTCUSDT orderbook depth...");

    // Run stream and event processing concurrently in the same task
    let stream_future = stream.ws_depth_orderbook("BTCUSDT", tx);

    tokio::select! {
        result = stream_future => {
            if let Err(e) = result {
                eprintln!("Stream error: {}", e);
            }
        }
        _ = async {
            let mut count = 0;
            while let Some(depth) = rx.recv().await {
                count += 1;
                let bids_preview: Vec<_> = depth.bids.iter().take(3).cloned().collect();
                let asks_preview: Vec<_> = depth.asks.iter().take(3).cloned().collect();
                println!(
                    "Orderbook update #{}: {} bids, {} asks (symbol: {})",
                    count,
                    bids_preview.len(),
                    asks_preview.len(),
                    depth.symbol
                );

                if count >= 3 {
                    break;
                }
            }
            println!("Received {} orderbook updates", count);
        } => {}
    }

    println!("Orderbook stream completed");
}

/// Example 3: Using the new Stream API for klines
async fn example_stream_klines() {
    let stream = Stream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<KlineEvent>();

    println!("Subscribing to ETHUSDT 1-minute klines...");

    // Run stream and event processing concurrently in the same task
    let stream_future = stream.ws_klines("ETHUSDT", "1m", tx);

    tokio::select! {
        result = stream_future => {
            if let Err(e) = result {
                eprintln!("Stream error: {}", e);
            }
        }
        _ = async {
            let mut count = 0;
            while let Some(kline_event) = rx.recv().await {
                count += 1;
                println!(
                    "Kline #{}: {} O:{}, H:{}, L:{}, C:{}",
                    count,
                    kline_event.symbol,
                    kline_event.kline.open,
                    kline_event.kline.high,
                    kline_event.kline.low,
                    kline_event.kline.close
                );

                if count >= 3 {
                    break;
                }
            }
            println!("Received {} kline updates", count);
        } => {}
    }

    println!("Kline stream completed");
}

/// Example 4: Using the new Stream API for ticker
async fn example_stream_ticker() {
    let stream = Stream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<DayTickerEvent>();

    println!("Subscribing to BTCUSDT 24hr ticker...");

    // Run stream and event processing concurrently in the same task
    let stream_future = stream.ws_ticker("BTCUSDT", tx);

    tokio::select! {
        result = stream_future => {
            if let Err(e) = result {
                eprintln!("Stream error: {}", e);
            }
        }
        _ = async {
            let mut count = 0;
            while let Some(ticker) = rx.recv().await {
                count += 1;
                println!(
                    "Ticker #{}: {} Price: {} Change: {}% Volume: {}",
                    count,
                    ticker.symbol,
                    ticker.current_close,
                    ticker.price_change_percent,
                    ticker.volume
                );

                if count >= 2 {
                    break;
                }
            }
            println!("Received {} ticker updates", count);
        } => {}
    }

    println!("Ticker stream completed");
}

/// Example 5: Using the new Stream API for user data with auto keep-alive
async fn example_stream_user_data_auto() {
    println!("Note: User data stream requires API credentials");
    println!("This example shows the API pattern but won't run without valid credentials\n");

    // This is a demonstration of the API pattern
    // In practice, you would need to provide a client with API credentials
    /*
    use binance::api::Binance;

    let api_key = Some("YOUR_API_KEY".to_string());
    let client = Binance::new(api_key, None);
    let stream = Stream::with_client(client);

    let (tx, mut rx) = mpsc::unbounded_channel::<WebsocketEvent>();

    // This will automatically:
    // 1. Start the user data stream
    // 2. Spawn a keep-alive task (every 30 minutes)
    // 3. Forward events to the channel
    if let Err(e) = stream.ws_user_data_auto(tx).await {
        eprintln!("Error setting up user data stream: {}", e);
        return;
    }

    println!("User data stream started with auto keep-alive");

    // Process events
    while let Some(event) = rx.recv().await {
        match event {
            WebsocketEvent::AccountUpdate(update) => {
                println!("Account update: {} balances", update.data.balances.len());
            }
            WebsocketEvent::OrderTrade(trade) => {
                println!("Order trade: {} {} @ {}",
                    trade.symbol, trade.side, trade.price);
            }
            _ => {}
        }
    }
    */

    println!("(User data example skipped - requires API credentials)");
}

/// Example 6: Using the new Stream API for multiple streams
async fn example_stream_multiple() {
    let stream = Stream::new();
    let (tx1, mut rx1) = mpsc::unbounded_channel::<TradeEvent>();
    let (tx2, mut rx2) = mpsc::unbounded_channel::<DayTickerEvent>();

    println!("Subscribing to multiple streams (BTCUSDT trades & ETHUSDT ticker)...");

    // Start both streams
    let stream1_future = stream.ws_trades("BTCUSDT", tx1);
    let stream2_future = stream.ws_ticker("ETHUSDT", tx2);

    // Process both streams concurrently
    let mut trade_count = 0;
    let mut ticker_count = 0;

    // Use a timeout to limit the example duration
    let timeout_future = sleep(Duration::from_secs(2));

    tokio::select! {
        result = stream1_future => {
            if let Err(e) = result {
                eprintln!("Stream 1 error: {}", e);
            }
        }
        result = stream2_future => {
            if let Err(e) = result {
                eprintln!("Stream 2 error: {}", e);
            }
        }
        _ = async {
            let start = tokio::time::Instant::now();
            while start.elapsed() < Duration::from_secs(2) {
                tokio::select! {
                    trade = rx1.recv() => {
                        if let Some(trade) = trade {
                            trade_count += 1;
                            if trade_count <= 2 {
                                println!("BTC Trade #{}: @ {}", trade_count, trade.price);
                            }
                        }
                    }
                    ticker = rx2.recv() => {
                        if let Some(ticker) = ticker {
                            ticker_count += 1;
                            if ticker_count <= 2 {
                                println!("ETH Ticker #{}: {}", ticker_count, ticker.current_close);
                            }
                        }
                    }
                    _ = sleep(Duration::from_millis(10)) => {
                        // Continue checking
                    }
                }

                if trade_count >= 2 && ticker_count >= 2 {
                    break;
                }
            }

            println!(
                "Received {} trades and {} ticker updates",
                trade_count, ticker_count
            );
        } => {}
        _ = timeout_future => {
            println!("Timeout reached for multiple streams example");
        }
    }

    println!("Multiple streams completed");
}

/// Example 7: Legacy WebSockets API for comparison
async fn example_legacy_websockets() {
    use binance::websockets::{WebSockets, WebsocketEvent as LegacyEvent};

    let keep_running = Arc::new(AtomicBool::new(true));
    let keep_running_clone = keep_running.clone();

    // Stop after 2 seconds
    tokio::spawn(async move {
        sleep(Duration::from_secs(2)).await;
        println!("Stopping legacy WebSocket...");
        keep_running_clone.store(false, Ordering::Relaxed);
    });

    let mut web_socket = WebSockets::new(|event: LegacyEvent| {
        match event {
            LegacyEvent::Trade(trade) => {
                println!(
                    "[Legacy] Trade: {} @ {} (qty: {})",
                    trade.symbol, trade.price, trade.qty
                );
            }
            _ => (),
        };
        Ok(())
    });

    println!("Connecting legacy WebSocket to BTCUSDT trades...");

    if let Err(e) = web_socket.connect("btcusdt@trade").await {
        println!("Connection error: {}", e);
        return;
    }

    println!("Running legacy event loop for 2 seconds...");
    if let Err(e) = web_socket.event_loop(&keep_running).await {
        println!("Event loop error: {}", e);
    }

    println!("Legacy WebSocket example completed");
}

/// Helper function to demonstrate testnet usage
async fn _example_testnet() {
    // Example using testnet
    let _stream = Stream::new();
    // Configure for testnet
    let config = binance::websockets::StreamConfig {
        market: Market::SpotTestnet,
        ..Default::default()
    };
    let _testnet_stream = Stream::with_config(config);

    println!("Testnet stream created (requires testnet credentials)");
    // Use testnet_stream.ws_trades(), etc.
}

/// Helper function to demonstrate market configuration
async fn _example_market_config() {
    println!("Stream API supports different markets:");
    println!("- Market::Spot (default)");
    println!("- Market::SpotTestnet");

    let _spot_stream = Stream::new(); // Defaults to Spot

    let testnet_config = binance::websockets::StreamConfig {
        market: Market::SpotTestnet,
        ..Default::default()
    };
    let _testnet_stream = Stream::with_config(testnet_config);

    println!("Different market configurations available");
}
