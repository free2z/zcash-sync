use clap::{App, Arg};
use std::fs::File;
use std::io::{Read, Write};
use sync::{KeyHelpers, Tx};
use zcash_client_backend::encoding::decode_extended_spending_key;
use zcash_primitives::consensus::{Network, Parameters};
use zcash_proofs::prover::LocalTxProver;
use zcash_params::coin::CoinType;

const NETWORK: Network = Network::MainNetwork; // TODO

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let key = dotenv::var("KEY").unwrap();
    let kh = KeyHelpers::new(CoinType::Zcash);
    let (_seed, sk, _ivk, _address) = kh.decode_key(&key)?;

    let matches = App::new("Multisig CLI")
        .version("1.0")
        .arg(
            Arg::with_name("tx_filename")
                .short("tx")
                .long("tx")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("out_filename")
                .short("o")
                .long("out")
                .takes_value(true),
        )
        .get_matches();

    let tx_filename = matches.value_of("tx_filename").unwrap();
    let out_filename = matches.value_of("out_filename").unwrap();

    let sk = sk.unwrap();
    let sk =
        decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &sk)?.unwrap();

    let mut file = File::open(tx_filename)?;
    let mut s = String::new();
    file.read_to_string(&mut s).unwrap();
    let tx: Tx = serde_json::from_str(&s)?;
    let prover = LocalTxProver::with_default_location()
        .ok_or_else(|| anyhow::anyhow!("Cannot create prover. Missing zcash-params?"))?;
    let raw_tx = tx.sign(None, &sk, &prover, |p| {
        println!("Progress {}", p.cur());
    })?;

    let mut out_file = File::create(out_filename)?;
    writeln!(out_file, "{}", hex::encode(&raw_tx))?;
    Ok(())
}
