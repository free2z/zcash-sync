// #![allow(non_snake_case)]
use node_bindgen::derive::node_bindgen;
use node_bindgen::init::node_bindgen_init_once;

use std::sync::atomic::AtomicBool;
use std::convert::TryInto;

use lazy_static::lazy_static;
use log::{info, warn};


// use rocket::serde::{json::Json, Deserialize, Serialize};
// use warp_api_ffi::{get_best_server, AccountRec, CoinConfig, RaptorQDrops, Tx, TxRec};
// use thiserror::Error;
// use anyhow::Error;

// TODO: logging!
// https://github.com/infinyon/node-bindgen/blob/master/examples/logging/src/lib.rs
// this doesn't seem to work :/
#[node_bindgen_init_once]
fn init_logging() {
    // initialize logging framework
    env_logger::init();
    info!("logging initialized");
}



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
            log::error!("{}", err);
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
    let coin = coin as u8;
    info!("Init coin");
    crate::init_coin(coin, &db_path).unwrap();
    crate::set_coin_lwd_url(coin, &lwd_url);
}

#[node_bindgen]
fn new_account(coin: u32, name: String) {
    info!("warp.new_account");
    crate::api::account::new_account(coin as u8, &name, None, None).unwrap();
}

#[node_bindgen]
fn set_active_account(coin: u32, id: u32) {
    warn!("warp.set_active_account");
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
async fn get_latest_height() -> i32 {
    info!("warp.get_latest_height");
    let height = crate::api::sync::get_latest_height().await;
    // let height = height as u32;
    log_result(height).try_into().unwrap()
}

lazy_static! {
    static ref SYNC_CANCELED: AtomicBool = AtomicBool::new(false);
}

// Does not support tokio async executor atm
#[tokio::main]
#[node_bindgen]
async fn warp(coin: u32, offset: u32) {
    info!("warp.warp started");
    // 0 == ZEC
    // true = get_tx
    //
    crate::api::sync::coin_sync(coin as u8, true, offset, move |height| {

    }, &SYNC_CANCELED)
        .await
        .unwrap();
}

#[node_bindgen]
fn get_lwd_url(coin: u32) -> String {
    info!("warp.get_lwd_url");
    let coin = coin as u8;
    return crate::coinconfig::get_coin_lwd_url(coin);
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