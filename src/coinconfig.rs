use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use lazy_static::lazy_static;
use lazycell::AtomicLazyCell;
use tonic::transport::Channel;
use zcash_proofs::prover::LocalTxProver;
use zcash_params::{coin, OUTPUT_PARAMS, SPEND_PARAMS};
use zcash_params::coin::{CoinChain, CoinType, get_coin_chain};
use crate::{CompactTxStreamerClient, connect_lightwalletd, DbAdapter, MemPool};

lazy_static! {
    pub static ref COIN_CONFIG: [Mutex<CoinConfig>; 2] = [
        Mutex::new(CoinConfig::new(CoinType::Zcash)),
        Mutex::new(CoinConfig::new(CoinType::Ycash)),
    ];
    pub static ref PROVER: AtomicLazyCell<LocalTxProver> = AtomicLazyCell::new();
}

pub static ACTIVE_COIN: AtomicU8 = AtomicU8::new(0);

pub fn set_active(active: u8) {
    ACTIVE_COIN.store(active, Ordering::Release);
}

pub fn set_active_account(coin: u8, id: u32) {
    let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.id_account = id;
}

pub fn set_coin_lwd_url(coin: u8, lwd_url: &str) {
    let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.lwd_url = lwd_url.to_string();
}

pub fn init_coin(coin: u8, db_path: &str) {
    let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.db_path = db_path.to_string();
}

#[derive(Clone)]
pub struct CoinConfig {
    pub coin_type: CoinType,
    pub id_account: u32,
    pub height: u32,
    pub lwd_url: String,
    pub db_path: String,
    pub mempool: Arc<Mutex<MemPool>>,
    pub chain: &'static (dyn CoinChain + Send),
}

impl CoinConfig {
    pub fn new(coin_type: CoinType) -> Self {
        let chain = get_coin_chain(coin_type);
        CoinConfig {
            coin_type,
            id_account: 0,
            height: 0,
            lwd_url: String::new(),
            db_path: String::new(),
            mempool: Arc::new(Mutex::new(MemPool::new())),
            chain,
        }
    }

    pub fn get(coin: u8) -> CoinConfig {
        let c = COIN_CONFIG[coin as usize].lock().unwrap();
        c.clone()
    }

    pub fn get_active() -> CoinConfig {
        let coin = ACTIVE_COIN.load(Ordering::Acquire) as usize;
        let c = COIN_CONFIG[coin].lock().unwrap();
        c.clone()
    }

    pub fn set_height(height: u32) {
        let coin = ACTIVE_COIN.load(Ordering::Acquire) as usize;
        let mut c = COIN_CONFIG[coin].lock().unwrap();
        c.height = height;
    }

    pub fn mempool(&self) -> MutexGuard<MemPool> {
        self.mempool.lock().unwrap()
    }

    pub fn db(&self) -> anyhow::Result<DbAdapter> {
        DbAdapter::new(self.coin_type, &self.db_path)
    }

    pub async fn connect_lwd(&self) -> anyhow::Result<CompactTxStreamerClient<Channel>> {
        connect_lightwalletd(&self.lwd_url).await
    }
}

pub fn get_prover() -> &'static LocalTxProver {
    if !PROVER.filled() {
        let _ = PROVER.fill(LocalTxProver::from_bytes(SPEND_PARAMS, OUTPUT_PARAMS));
    }
    PROVER.borrow().unwrap()
}

