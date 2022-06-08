use std::sync::Arc;
use anyhow::anyhow;
use bip39::{Language, Mnemonic};
use rand::prelude::*;
use rand::rngs::OsRng;
use tokio::sync::Mutex;
use tonic::Request;
use tonic::transport::Channel;
use crate::coinconfig::{ACTIVE_COIN, CoinConfig};
use crate::{BlockId, CompactTxStreamerClient, connect_lightwalletd, CTree};
use crate::key2::{decode_key, is_valid_key};
use crate::scan::AMProgressCallback;

pub mod account;
pub mod sync;
pub mod payment;
pub mod contact;
pub mod message;
pub mod fullbackup;
pub mod historical_prices;
pub mod payment_uri;
pub mod mempool;

pub mod dart_ffi;
