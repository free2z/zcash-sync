#![allow(non_snake_case)]
use lazy_static::lazy_static;
use node_bindgen::derive::node_bindgen;
use std::sync::atomic::AtomicBool;


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
fn initCoin(coin: u32, db_path: String, lwd_url: String) {
    let coin = coin as u8;
    log::info!("Init coin");
    crate::init_coin(coin, &db_path).unwrap();
    crate::set_coin_lwd_url(coin, &lwd_url);
}

#[node_bindgen]
fn newAccount(coin: u32, name: String) {
    crate::api::account::new_account(coin as u8, &name, None, None).unwrap();
}

lazy_static! {
    static ref SYNC_CANCELED: AtomicBool = AtomicBool::new(false);
}

// Does not support tokio async executor atm
#[tokio::main]
#[node_bindgen]
async fn warp(coin: u32) {
    crate::api::sync::coin_sync(coin as u8, true, 0, move |height| {}, &SYNC_CANCELED)
        .await
        .unwrap();
}

#[node_bindgen]
fn getLWDURL(coin: u32) -> String {
    let coin = coin as u8;
    return crate::coinconfig::get_coin_lwd_url(coin);
}

#[node_bindgen]
fn isValidAddress(coin: u32, address: String) -> bool {
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