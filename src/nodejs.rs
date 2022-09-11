// #![allow(non_snake_case)]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::convert::TryInto;

use node_bindgen::derive::node_bindgen;
use node_bindgen::core::NjError;
use node_bindgen::init::node_bindgen_init_once;

use lazy_static::lazy_static;
// use log::{info, warn};

// use rocket::serde::{json::Json, Deserialize, Serialize};
// use warp_api_ffi::{get_best_server, AccountRec, CoinConfig, RaptorQDrops, Tx, TxRec};
// use thiserror::Error;
// use anyhow::Error;

// use crate::wallet::RecipientMemo;
use crate::api::payment::RecipientMemo;
// use crate::{ChainError, Tx};
use crate::{ChainError};

use log::{info, warn, error};


#[node_bindgen_init_once]
fn init_logging() {
    // initialize logging framework
    env_logger::init();
    info!("logging initialized");
}

// TODO: logging!
// https://github.com/infinyon/node-bindgen/blob/master/examples/logging/src/lib.rs
// this doesn't seem to work :/
// newer feature in 5.X I think
// #[node_bindgen_init_once]
// fn init_logging() {
//     // initialize logging framework
//     env_logger::init();
//     info!("logging initialized");
// }



fn log_result<T: Default>(result: anyhow::Result<T>) -> T {
    match result {
        Err(err) => {
            log::error!("{}", err);
            // let last_error = LAST_ERROR.lock().unwrap();
            // last_error.replace(err.to_string());
            // IS_ERROR.store(true, Ordering::Release);
            T::default()
        }
        Ok(v) => {
            // IS_ERROR.store(false, Ordering::Release);
            v
        }
    }
}



fn log_string(result: anyhow::Result<String>) -> String {
    match result {
        Err(err) => {
            log::error!("{}!!!", err);
            // let last_error = LAST_ERROR.lock().unwrap();
            // last_error.replace(err.to_string());
            // IS_ERROR.store(true, Ordering::Release);
            format!("{}", err)
        }
        Ok(v) => {
            // IS_ERROR.store(false, Ordering::Release);
            v
        }
    }
}


#[node_bindgen]
fn init_coin(coin: u32, db_path: String, lwd_url: String) {
    warn!("calling init_coin");

    let coin = coin as u8;
    // info!("Init coin");
    crate::init_coin(coin, &db_path).unwrap();
    crate::set_coin_lwd_url(coin, &lwd_url);
}

#[node_bindgen]
fn new_account(coin: u32, name: String) {
    // info!("warp.new_account");
    crate::api::account::new_account(coin as u8, &name, None, None).unwrap();
}

#[node_bindgen]
fn set_active_account(coin: u32, id: u32) {
    // warn!("warp.set_active_account");
    crate::coinconfig::set_active_account(coin as u8, id);
}

// //
// #[node_bindgen]
// fn list_accounts() -> Result<Json<Vec<AccountRec>>, Error> {
//     let c = CoinConfig::get_active();
//     let db = c.db()?;
//     let accounts = db.get_accounts()?;
//     Ok(Json(accounts))
// }


// #[node_bindgen]
// fn list_accounts() {
//     let cc = CoinConfig::get_active();
//     // let cc = crate::coinconfig::get_active();
//     let db = cc.db();
//     // let accounts = db.get_accounts();
//     // accounts
// }

#[tokio::main]
#[node_bindgen]
async fn get_server_height() -> i32 {
    // info!("warp.get_latest_height");
    let height = crate::api::sync::get_latest_height().await;
    // let height = height as u32;
    log_result(height).try_into().unwrap()
}

#[tokio::main]
#[node_bindgen]
pub async fn skip_to_last_height() {
    let res = crate::api::sync::skip_to_last_height(0 as u8).await;
    log_result(res)
}


#[tokio::main]
#[node_bindgen]
async fn get_sync_height() -> i32 {
    // info!("warp.get_latest_height");
    let height = crate::api::sync::get_synced_height();
    // let height = height as u32;
    log_result(height).try_into().unwrap()
}


// #[tokio::main]
// #[node_bindgen]
// async fn rewind_to_height(height: u32) {
//     let res = crate::wallet::rewind_to_height(height).await;
//     log_result(res)
// }


lazy_static! {
    // static ref SYNC_CANCELED: AtomicBool = AtomicBool::new(false);
    static ref SYNC_CANCELED: Mutex<bool> = Mutex::new(false);

    // static ref WARP_OFFSET: AtomicU32 = AtomicU32::new(0);
}

// TODO
// This just hands for a while and then errors
// $ RUST_BACKTRACE=full node
// Welcome to Node.js v18.2.0.
// Type ".help" for more information.
// > const warp = require("./dist/index.node")
// undefined
// > warp.initCoin(0, "./zec.db", "https://zuul.free2z.cash:9067")
// undefined
// > warp.warpcb(cb)
// Uncaught ReferenceError: cb is not defined
// > cb = function(int) { console.log(int) }
// [Function: cb]
// > warp.warpcb(cb)
//
// # Runtime error!
//
// # Fatal error in HandleScope::HandleScope
// # Entering the V8 API without proper locking in place
// #
//
// #[tokio::main]
// #[node_bindgen]
// async fn warpcb<F: Fn(u32)>(cb: F) {
//     // let F = F as Fn(u32) + std::marker::Send + 'static;
//     let cb = cb as (dyn Fn(u32) + std::marker::Send + 'static);
//     crate::api::sync::coin_sync(
//         0, true, 0,
//         cb,
//         &SYNC_CANCELED)
//         .await
//         .unwrap();
// }

// #[node_bindgen]
// fn get_sync_height() -> u32 {
//     // *WARP_OFFSET.get_mut()
//     // *WARP_OFFSET as u32
//     WARP_OFFSET.load(Ordering::Relaxed)
// }

#[tokio::main]
#[node_bindgen]
async fn warp() {
    crate::api::sync::coin_sync(
        // zec, use transparent, 0 offset, filter > 20 max_cost
        0, true, 0, 20,
        move |_height| {
        //
        },
        &SYNC_CANCELED
    ).await.unwrap();
}


// Does not support tokio async executor atm
#[tokio::main]
#[node_bindgen]
async fn warp_handle() -> u8 {
    error!("Calling warp!");
    warn!("warn");
    info!("yo info");

    // YOU MUST initCoin first!!!
    let res = async {
        let result = crate::api::sync::coin_sync(
            // zec, use transparent, 0 offset, filter max_cost>20
            0, true, 0, 20,
            move |_height| {
            //
            },
            &SYNC_CANCELED
        ).await;

        // what's this about?
        // need an active account to run this one?
        // crate::api::mempool::scan().await?;

        match result {
            Ok(_) => Ok(0),
            Err(err) => {
                if let Some(e) = err.downcast_ref::<ChainError>() {
                    match e {
                        ChainError::Reorg => Ok(1),
                        ChainError::Busy => Ok(2),
                    }
                } else {
                    log::error!("Non-chain error: {}", err);
                    // 11111111
                    Ok(0xFF)
                }
            }
        }
    };
    let r = res.await;
    // *SYNC_CANCELED.store(false, Ordering::Release);
    log_result(r)
}

// This would only be relevant if we could use the same process?
// But, I think we will just tell the main process to kill the warp,
// rewind it and start again? Might be a little bit rude?
// But, hrmmm ...
// #[node_bindgen]
// pub unsafe extern "C" fn cancel_warp() {
//     log::info!("Sync canceled");
//     SYNC_CANCELED.store(true, Ordering::Release);
// }


// TODO: this is still not async ...
// I think we have to go to a lower level with scan::sync_async
// But, that's at a lower level. Maybe later.
// For now the warp sync in a fork seems _pretty_ solid :shruggie:
// it's weird though after a bunch of gymnastics here, the
// result is still undefined lol
// > p = warp.prometo(0)
// undefined
#[tokio::main]
#[node_bindgen]
async fn prometo(offset: u32) -> Result<(), NjError> {
    crate::api::sync::coin_sync(
        0, true, offset, 20, move |_height| {}, &SYNC_CANCELED
    ).await.map_err(|e| NjError::Other(format!("{}", e)))
}

// struct TestObject {
//     val: Option<f64>,
// }

// #[node_bindgen]
// impl RecipientMemo {
//     #[node_bindgen(constructor)]
//     fn new() -> Self {
//         Self { val: None }
//     }


#[tokio::main]
#[node_bindgen]
async fn send_multi_payment(
    // recipients: &[RecipientMemo],
    recipients_json: String,
    // use_transparent: bool,
    // anchor_offset: u32,
    // port: i64,
) -> String {
    // from_c_str!(recipients_json);
    let res = async move {
        let height = crate::api::sync::get_latest_height().await?;
        let recipients = crate::api::payment::parse_recipients(&recipients_json)?;

        // TODO: just send in already parsed?
        // let recipients = crate::api::payment::parse_recipients(&recipients_json)?;
        let res = crate::api::payment::build_sign_send_multi_payment(
            height,
            &recipients,
            false, // use_transparent,
            0,  // anchor offset
            Box::new(move |_progress| {
                // report_progress(progress, port);
            }),
        )
        .await?;
        Ok(res)
    };
    log_string(res.await)
}



#[node_bindgen]
fn get_lwd_url(coin: u32) -> String {
    // info!("warp.get_lwd_url");
    let coin = coin as u8;
    return crate::coinconfig::get_coin_lwd_url(coin);
}

#[tokio::main]
#[node_bindgen]
async fn get_block_by_time(time: u32) -> u32 {
    crate::api::sync::get_block_by_time(time).await.unwrap()
}

#[node_bindgen]
fn is_valid_address(coin: u32, address: String) -> bool {
    let coin = coin as u8;
    crate::key2::is_valid_address(coin, &address)
}

#[node_bindgen]
fn make_payment_uri(
    address: String,
    amount: u32,
    memo: String,
) -> String {
    let amount = amount as u64;
    let res = crate::api::payment_uri::make_payment_uri(&address, amount, &memo);
    log_string(res)
}