//! Modern Futures WebSocket API example for Binance
//!
//! This example demonstrates the new FuturesStream API implementation with:
//! 1. Channel-based output instead of callbacks
//! 2. Support for all futures market types (USD-M, COIN-M, Vanilla)
//! 3. Futures-specific streams (mark price, liquidations, continuous klines)
//! 4. Automatic keep-alive for user data streams
//! 5. Comparison with legacy FuturesWebSockets API

use binance::{
    futures::{FuturesMarket, FuturesWebSockets, FuturesWebsocketEvent, websockets::FuturesStream},
    model::{
        AggrTradesEvent, ContinuousKlineEvent, DayTickerEvent, LiquidationEvent, MarkPriceEvent
    },
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    println!("=== Modern Binance Futures WebSocket Example ===\n");

    // Example 1: New FuturesStream API - USD-M Trades
    println!("1. New FuturesStream API - USD-M Trades stream:");
    example_futures_stream_usdm_trades().await;

    // Example 2: New FuturesStream API - Mark Price
    println!("\n2. New FuturesStream API - Mark Price stream:");
    example_futures_stream_mark_price().await;

    // Example 3: New FuturesStream API - Liquidations
    println!("\n3. New FuturesStream API - Liquidations stream:");
    example_futures_stream_liquidations().await;

    // Example 4: New FuturesStream API - Continuous Klines
    println!("\n4. New FuturesStream API - Continuous Klines:");
    example_futures_stream_continuous_klines().await;

    // Example 5: New FuturesStream API - Multiple Markets
    println!("\n5. New FuturesStream API - Multiple Markets:");
    example_futures_stream_multiple_markets().await;

    // Example 6: New FuturesStream API - User Data with Auto Keep-alive
    println!("\n6. New FuturesStream API - User Data stream (with auto keep-alive):");
    example_futures_stream_user_data_auto().await;

    // Example 7: Legacy FuturesWebSockets API for comparison
    println!("\n7. Legacy FuturesWebSockets API (for comparison):");
    example_legacy_futures_websockets().await;

    println!("\n=== All examples completed ===");
}

/// Example 1: Using the new FuturesStream API for USD-M trades
async fn example_futures_stream_usdm_trades() {
    let stream = FuturesStream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<AggrTradesEvent>();

    println!("Subscribing to BTCUSDT aggregated trades on USD-M...");

    // Run stream and event processing concurrently in the same task
    let stream_future = stream.ws_agg_trades("BTCUSDT", tx);

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
                    "USD-M Trade #{}: {} @ {} (qty: {}) - Time: {}",
                    count, trade.symbol, trade.price, trade.qty, trade.event_time
                );
                if count >= 5 {
                    break;
                }
            }
            println!("Received {} USD-M trades", count);
        } => {}
    }

    println!("USD-M trades stream completed");
}

/// Example 2: Using the new FuturesStream API for mark price
async fn example_futures_stream_mark_price() {
    let stream = FuturesStream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<MarkPriceEvent>();

    println!("Subscribing to BTCUSDT mark price stream on USD-M...");

    // Run stream and event processing concurrently in the same task
    let stream_future = stream.ws_mark_price("BTCUSDT", tx);

    tokio::select! {
        result = stream_future => {
            if let Err(e) = result {
                eprintln!("Stream error: {}", e);
            }
        }
        _ = async {
            let mut count = 0;
            while let Some(mark_price) = rx.recv().await {
                count += 1;
                println!(
                    "Mark Price #{}: {} Index: {:?} Mark: {} Funding: {} Next: {}",
                    count,
                    mark_price.symbol,
                    mark_price.index_price,
                    mark_price.mark_price,
                    mark_price.funding_rate,
                    mark_price.next_funding_time
                );
                if count >= 3 {
                    break;
                }
            }
            println!("Received {} mark price updates", count);
        } => {}
    }

    println!("Mark price stream completed");
}

/// Example 3: Using the new FuturesStream API for liquidations
async fn example_futures_stream_liquidations() {
    let stream = FuturesStream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<LiquidationEvent>();

    println!("Subscribing to BTCUSDT liquidations stream on USD-M...");

    // Run stream and event processing concurrently in the same task
    let stream_future = stream.ws_liquidations("BTCUSDT", tx);

    tokio::select! {
        result = stream_future => {
            if let Err(e) = result {
                eprintln!("Stream error: {}", e);
            }
        }
        _ = async {
            let mut count = 0;
            while let Some(liquidation) = rx.recv().await {
                count += 1;
                println!(
                    "Liquidation #{}: {} Side: {} Price: {} Qty: {}",
                    count,
                    liquidation.liquidation_order.symbol,
                    liquidation.liquidation_order.side,
                    liquidation.liquidation_order.price,
                    liquidation.liquidation_order.original_quantity
                );
                if count >= 2 {
                    break;
                }
            }
            if count == 0 {
                println!("No liquidations received (normal - depends on market conditions)");
            } else {
                println!("Received {} liquidation events", count);
            }
        } => {}
    }

    println!("Liquidations stream completed");
}

/// Example 4: Using the new FuturesStream API for continuous klines
async fn example_futures_stream_continuous_klines() {
    let stream = FuturesStream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<ContinuousKlineEvent>();

    println!("Subscribing to BTCUSDT continuous klines (perpetual) on USD-M...");

    // Run stream and event processing concurrently in the same task
    let stream_future = stream.ws_continuous_klines("BTCUSDT", "perpetual", "1m", tx);

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
                    "Continuous Kline #{}: Pair: {} Contract: {} O:{}, H:{}, L:{}, C:{}",
                    count,
                    kline_event.pair,
                    kline_event.contract_type,
                    kline_event.kline.open,
                    kline_event.kline.high,
                    kline_event.kline.low,
                    kline_event.kline.close
                );
                if count >= 2 {
                    break;
                }
            }
            println!("Received {} continuous kline updates", count);
        } => {}
    }

    println!("Continuous klines stream completed");
}

/// Example 5: Using the new FuturesStream API with multiple markets
async fn example_futures_stream_multiple_markets() {
    println!("Demonstrating different futures market configurations:");

    // Example with default configuration (USD-M)
    let usdm_stream = FuturesStream::new();
    println!("- Default stream: USD-M market");

    // Example with COIN-M configuration
    let coinm_config = binance::futures::websockets::FuturesStreamConfig {
        market: FuturesMarket::COINM,
        ..Default::default()
    };
    let _coinm_stream = FuturesStream::with_config(coinm_config);
    println!("- COIN-M stream configured");

    // Example with testnet
    let testnet_config = binance::futures::websockets::FuturesStreamConfig {
        market: FuturesMarket::USDMTestnet,
        ..Default::default()
    };
    let _testnet_stream = FuturesStream::with_config(testnet_config);
    println!("- USD-M Testnet stream configured");

    // Example with Vanilla options
    let vanilla_config = binance::futures::websockets::FuturesStreamConfig {
        market: FuturesMarket::Vanilla,
        ..Default::default()
    };
    let _vanilla_stream = FuturesStream::with_config(vanilla_config);
    println!("- Vanilla options stream configured");

    println!("\nAll market configurations available:");
    println!("- FuturesMarket::USDM (default)");
    println!("- FuturesMarket::COINM");
    println!("- FuturesMarket::Vanilla");
    println!("- FuturesMarket::USDMTestnet");
    println!("- FuturesMarket::COINMTestnet");
    println!("- FuturesMarket::VanillaTestnet");

    // Simple demonstration with USD-M stream using correct channel type
    let (tx, mut rx) = mpsc::unbounded_channel::<DayTickerEvent>();

    // Run stream and event processing concurrently in the same task
    let stream_future = usdm_stream.ws_ticker("BTCUSDT", tx);

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
                    "USD-M Ticker #{}: {} Price: {}",
                    count, ticker.symbol, ticker.current_close
                );
                if count >= 1 {
                    break;
                }
            }
            if count > 0 {
                println!("Received {} USD-M ticker update(s)", count);
            }
        } => {}
    }

    println!("Multiple markets demonstration completed");
}

/// Example 6: Using the new FuturesStream API for user data with auto keep-alive
async fn example_futures_stream_user_data_auto() {
    println!("Note: Futures user data stream requires API credentials");
    println!("This example shows the API pattern but won't run without valid credentials\n");

    // This is a demonstration of the API pattern
    // In practice, you would need to provide a client with API credentials
    /*
    use binance::api::Binance;
    use binance::futures::userstream::FuturesUserStream;

    let api_key = Some("YOUR_API_KEY".to_string());
    let client = Binance::new(api_key, None);
    let stream = FuturesStream::with_client(client);

    let (tx, mut rx) = mpsc::unbounded_channel::<FuturesWebsocketEvent>();

    // This will automatically:
    // 1. Start the futures user data stream
    // 2. Spawn a keep-alive task
    // 3. Forward events to the channel
    if let Err(e) = stream.ws_user_data_auto(tx).await {
        eprintln!("Error setting up futures user data stream: {}", e);
        return;
    }

    println!("Futures user data stream started with auto keep-alive");

    // Process events
    while let Some(event) = rx.recv().await {
        match event {
            FuturesWebsocketEvent::AccountUpdate(update) => {
                println!("Futures Account update: {} positions", update.data.positions.len());
            }
            FuturesWebsocketEvent::OrderTrade(trade) => {
                println!("Futures Order trade: {} {} @ {}",
                    trade.symbol, trade.side, trade.price);
            }
            FuturesWebsocketEvent::UserDataStreamExpiredEvent(expired) => {
                println!("User data stream expired: {}", expired.listen_key);
            }
            _ => {}
        }
    }
    */

    println!("(Futures user data example skipped - requires API credentials)");
}

/// Example 7: Legacy FuturesWebSockets API for comparison
async fn example_legacy_futures_websockets() {
    let keep_running = Arc::new(AtomicBool::new(true));
    let keep_running_clone = keep_running.clone();

    // Stop after 2 seconds
    tokio::spawn(async move {
        sleep(Duration::from_secs(2)).await;
        println!("Stopping legacy Futures WebSocket...");
        keep_running_clone.store(false, Ordering::Relaxed);
    });

    let mut web_socket = FuturesWebSockets::new(|event: FuturesWebsocketEvent| {
        match event {
            FuturesWebsocketEvent::AggrTrades(trade) => {
                println!(
                    "[Legacy] Futures Trade: {} @ {} (qty: {})",
                    trade.symbol, trade.price, trade.qty
                );
            }
            _ => (),
        };
        Ok(())
    });

    println!("Connecting legacy Futures WebSocket to BTCUSDT aggregated trades on USD-M...");

    if let Err(e) = web_socket
        .connect(&FuturesMarket::USDM, "btcusdt@aggTrade")
        .await
    {
        println!("Connection error: {}", e);
        return;
    }

    println!("Running legacy event loop for 2 seconds...");
    if let Err(e) = web_socket.event_loop(&keep_running).await {
        println!("Event loop error: {}", e);
    }

    println!("Legacy Futures WebSocket example completed");
}
