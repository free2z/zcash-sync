use crate::{connect_lightwalletd, CompactTxStreamerClient, DbAdapter, TxFilter, NETWORK};
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use tonic::transport::Channel;
use tonic::Request;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, encode_payment_address, encode_transparent_address,
};
use zcash_primitives::consensus::{BlockHeight, Parameters};
use zcash_primitives::memo::{Memo, MemoBytes};
use zcash_primitives::sapling::note_encryption::{
    try_sapling_note_decryption, try_sapling_output_recovery,
};
use zcash_primitives::transaction::Transaction;

#[derive(Debug)]
pub struct TransactionInfo {
    pub address: String,
    pub memo: Memo,
    amount: i64,
    pub fee: u64,
}

pub async fn decode_transaction(
    client: &mut CompactTxStreamerClient<Channel>,
    nfs: &HashMap<(u32, Vec<u8>), u64>,
    account: u32,
    fvk: &str,
    tx_hash: &[u8],
    height: u32,
) -> anyhow::Result<TransactionInfo> {
    let fvk =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &fvk)?
            .unwrap();
    let ivk = fvk.fvk.vk.ivk();
    let ovk = fvk.fvk.ovk;

    let tx_filter = TxFilter {
        block: None,
        index: 0,
        hash: tx_hash.to_vec(), // only hash is supported
    };
    let raw_tx = client
        .get_transaction(Request::new(tx_filter))
        .await?
        .into_inner();
    let tx = Transaction::read(&*raw_tx.data)?;

    let height = BlockHeight::from_u32(height);
    let mut amount = 0i64;
    let mut address = String::new();
    for spend in tx.shielded_spends.iter() {
        let nf = spend.nullifier.to_vec();
        if let Some(&v) = nfs.get(&(account, nf)) {
            amount -= v as i64;
        }
    }

    let mut tx_memo = MemoBytes::empty();
    for output in tx.vout.iter() {
        if let Some(t_address) = output.script_pubkey.address() {
            address = encode_transparent_address(
                &NETWORK.b58_pubkey_address_prefix(),
                &NETWORK.b58_script_address_prefix(),
                &t_address,
            );
        }
    }

    for output in tx.shielded_outputs.iter() {
        if let Some((note, pa, memo)) = try_sapling_note_decryption(&NETWORK, height, &ivk, output)
        {
            amount += note.value as i64; // change or self transfer
            if address.is_empty() {
                address = encode_payment_address(NETWORK.hrp_sapling_payment_address(), &pa);
                tx_memo = memo;
            }
        } else if let Some((_note, pa, memo)) =
            try_sapling_output_recovery(&NETWORK, height, &ovk, &output)
        {
            address = encode_payment_address(NETWORK.hrp_sapling_payment_address(), &pa);
            tx_memo = memo;
        }
    }

    let fee = u64::from(tx.value_balance);

    let tx_info = TransactionInfo {
        address,
        memo: Memo::try_from(tx_memo)?,
        amount,
        fee,
    };

    Ok(tx_info)
}

pub async fn retrieve_tx_info(tx_ids: &[u32], ld_url: &str, db_path: &str) -> anyhow::Result<()> {
    let mut tx_ids_set: HashSet<u32> = HashSet::new();
    for &tx_id in tx_ids.iter() {
        tx_ids_set.insert(tx_id);
    }
    let mut client = connect_lightwalletd(ld_url).await?;
    let db = DbAdapter::new(db_path)?;
    let nfs = db.get_nullifiers_raw()?;
    let mut nf_map: HashMap<(u32, Vec<u8>), u64> = HashMap::new();
    for nf in nfs.iter() {
        nf_map.insert((nf.0, nf.2.clone()), nf.1);
    }
    for &id_tx in tx_ids_set.iter() {
        let (account, height, tx_hash) = db.get_txhash(id_tx)?;
        let fvk = db.get_ivk(account)?;
        let tx_info =
            decode_transaction(&mut client, &nf_map, account, &fvk, &tx_hash, height).await?;
        db.store_tx_metadata(id_tx, &tx_info)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::transaction::decode_transaction;
    use crate::{connect_lightwalletd, DbAdapter, LWD_URL};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_decode_transaction() {
        let tx_hash =
            hex::decode("b47da170329dc311b98892eac23e83025f8bb3ce10bb07535698c91fb37e1e54")
                .unwrap();
        let mut client = connect_lightwalletd(LWD_URL).await.unwrap();
        let db = DbAdapter::new("./zec.db").unwrap();
        let account = 1;
        let nfs = db.get_nullifiers_raw().unwrap();
        let mut nf_map: HashMap<(u32, Vec<u8>), u64> = HashMap::new();
        for nf in nfs.iter() {
            if nf.0 == account {
                nf_map.insert((nf.0, nf.2.clone()), nf.1);
            }
        }
        let fvk = db.get_ivk(account).unwrap();
        let tx_info = decode_transaction(&mut client, &nf_map, account, &fvk, &tx_hash, 1313212)
            .await
            .unwrap();
        println!("{:?}", tx_info);
    }
}
