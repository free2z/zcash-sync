use crate::builder::BlockProcessor;
use crate::chain::{Nf, NfRef};
use crate::db::{DbAdapter, ReceivedNote};
use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
use crate::{
    calculate_tree_state_v2, connect_lightwalletd, download_chain, get_latest_height, CompactBlock,
    DecryptNode, Witness, LWD_URL, NETWORK,
};
use ff::PrimeField;
use log::{debug, info};
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::consensus::{NetworkUpgrade, Parameters};
use zcash_primitives::sapling::Node;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use std::panic;
use std::collections::HashMap;

pub async fn scan_all(fvks: &[ExtendedFullViewingKey]) -> anyhow::Result<()> {
    let fvks: HashMap<_, _> = fvks.iter().enumerate().map(|(i, fvk)|
        (i as u32, fvk.clone())).collect();
    let decrypter = DecryptNode::new(fvks);

    let total_start = Instant::now();
    let mut client = CompactTxStreamerClient::connect(LWD_URL).await?;
    let start_height: u32 = crate::NETWORK
        .activation_height(NetworkUpgrade::Sapling)
        .unwrap()
        .into();
    let end_height = get_latest_height(&mut client).await?;

    let start = Instant::now();
    let cbs = download_chain(&mut client, start_height, end_height, None).await?;
    info!("Download chain: {} ms", start.elapsed().as_millis());

    let start = Instant::now();
    let blocks = decrypter.decrypt_blocks(&cbs);
    info!("Decrypt Notes: {} ms", start.elapsed().as_millis());

    let witnesses = calculate_tree_state_v2(&cbs, &blocks);

    debug!("# Witnesses {}", witnesses.len());
    for w in witnesses.iter() {
        let mut bb: Vec<u8> = vec![];
        w.write(&mut bb)?;
        log::debug!("{}", hex::encode(&bb));
    }

    info!("Total: {} ms", total_start.elapsed().as_millis());

    Ok(())
}

struct Blocks(Vec<CompactBlock>);
struct BlockMetadata {
    height: u32,
    hash: [u8; 32],
    timestamp: u32,
}

impl std::fmt::Debug for Blocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blocks of len {}", self.0.len())
    }
}

pub type ProgressCallback = Arc<Mutex<dyn Fn(u32) + Send>>;

pub async fn sync_async(
    chunk_size: u32,
    db_path: &str,
    target_height_offset: u32,
    progress_callback: ProgressCallback,
    ld_url: &str
) -> anyhow::Result<()> {
    let ld_url = ld_url.to_owned();
    let db_path = db_path.to_string();

    let mut client = connect_lightwalletd(&ld_url).await?;
    let (start_height, mut prev_hash, fvks) = {
        let db = DbAdapter::new(&db_path)?;
        let height = db.get_db_height()?;
        let hash = db.get_db_hash(height)?;
        let fvks = db.get_fvks()?;
        (height, hash, fvks)
    };
    let end_height = get_latest_height(&mut client).await?;
    let end_height = (end_height - target_height_offset).max(start_height);

    let fvks: HashMap<_, _> = fvks.iter().map(|(&account, fvk)| {
        let fvk =
            decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &fvk)
                .unwrap()
                .unwrap();
        (account, fvk)
    }).collect();
    let decrypter = DecryptNode::new(fvks);

    let (downloader_tx, mut download_rx) = mpsc::channel::<Range<u32>>(2);
    let (processor_tx, mut processor_rx) = mpsc::channel::<Blocks>(1);

    let downloader = tokio::spawn(async move {
        let mut client = connect_lightwalletd(&ld_url).await?;
        while let Some(range) = download_rx.recv().await {
            log::info!("+ {:?}", range);
            let blocks = download_chain(&mut client, range.start, range.end, prev_hash).await?;
            log::debug!("- {:?}", range);
            blocks.last().map(|cb| {
                let mut ph = [0u8; 32];
                ph.copy_from_slice(&cb.hash);
                prev_hash = Some(ph);
            });
            let b = Blocks(blocks);
            processor_tx.send(b).await?;
        }
        log::info!("download completed");
        drop(processor_tx);

        Ok::<_, anyhow::Error>(())
    });

    let proc_callback = progress_callback.clone();

    let processor = tokio::spawn(async move {
        let db = DbAdapter::new(&db_path)?;
        let mut nfs = db.get_nullifiers()?;

        let (mut tree, mut witnesses) = db.get_tree()?;
        let mut bp = BlockProcessor::new(&tree, &witnesses);
        let mut absolute_position_at_block_start = tree.get_position();
        let mut last_block: Option<BlockMetadata> = None;
        while let Some(blocks) = processor_rx.recv().await {
            log::info!("{:?}", blocks);
            if blocks.0.is_empty() {
                continue;
            }

            let dec_blocks = decrypter.decrypt_blocks(&blocks.0);
            let mut witnesses: Vec<Witness> = vec![];
            for b in dec_blocks.iter() {
                let mut my_nfs: Vec<Nf> = vec![];
                for nf in b.spends.iter() {
                    if let Some(&nf_ref) = nfs.get(nf) {
                        log::info!("NF FOUND {} {}", nf_ref.id_note, b.height);
                        db.mark_spent(nf_ref.id_note, b.height)?;
                        my_nfs.push(*nf);
                    }
                }
                if !b.notes.is_empty() {
                    log::debug!("{} {}", b.height, b.notes.len());
                }

                for n in b.notes.iter() {
                    let p = absolute_position_at_block_start + n.position_in_block;

                    let note = &n.note;
                    let rcm = note.rcm().to_repr();
                    let nf = note.nf(&n.ivk.fvk.vk, p as u64);

                    let id_tx = db.store_transaction(
                        &n.txid,
                        n.account,
                        n.height,
                        b.compact_block.time,
                        n.tx_index as u32,
                    )?;
                    let id_note = db.store_received_note(
                        &ReceivedNote {
                            account: n.account,
                            height: n.height,
                            output_index: n.output_index as u32,
                            diversifier: n.pa.diversifier().0.to_vec(),
                            value: note.value,
                            rcm: rcm.to_vec(),
                            nf: nf.0.to_vec(),
                            spent: None,
                        },
                        id_tx,
                        n.position_in_block,
                    )?;
                    db.add_value(id_tx, note.value as i64)?;
                    nfs.insert(Nf(nf.0), NfRef { id_note, account: n.account });

                    let w = Witness::new(p as usize, id_note, Some(n.clone()));
                    witnesses.push(w);
                }

                if !my_nfs.is_empty() {
                    for (tx_index, tx) in b.compact_block.vtx.iter().enumerate() {
                        for cs in tx.spends.iter() {
                            let mut nf = [0u8; 32];
                            nf.copy_from_slice(&cs.nf);
                            let nf = Nf(nf);
                            if my_nfs.contains(&nf) {
                                let (account, note_value) = db.get_received_note_value(&nf)?;
                                let txid = &*tx.hash;
                                let id_tx = db.store_transaction(
                                    txid,
                                    account,
                                    b.height,
                                    b.compact_block.time,
                                    tx_index as u32,
                                )?;
                                db.add_value(id_tx, -(note_value as i64))?;
                            }
                        }
                    }
                }

                absolute_position_at_block_start += b.count_outputs as usize;
            }

            let mut nodes: Vec<Node> = vec![];
            for cb in blocks.0.iter() {
                for tx in cb.vtx.iter() {
                    for co in tx.outputs.iter() {
                        let mut cmu = [0u8; 32];
                        cmu.copy_from_slice(&co.cmu);
                        let node = Node::new(cmu);
                        nodes.push(node);
                    }
                }
            }

            if !nodes.is_empty() {
                bp.add_nodes(&mut nodes, &witnesses);
            }
            // println!("NOTES = {}", nodes.len());

            if let Some(block) = blocks.0.last() {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&block.hash);
                last_block = Some(BlockMetadata {
                    height: block.height as u32,
                    hash,
                    timestamp: block.time,
                });
            }
            log::info!("progress: {}", blocks.0[0].height);
            let callback = proc_callback.lock().await;
            callback(blocks.0[0].height as u32);
        }

        // Finalize scan
        let (new_tree, new_witnesses) = bp.finalize();
        tree = new_tree;
        witnesses = new_witnesses;

        if let Some(last_block) = last_block {
            let last_height = last_block.height;
            db.store_block(last_height, &last_block.hash, last_block.timestamp, &tree)?;
            for w in witnesses.iter() {
                db.store_witnesses(w, last_height, w.id_note)?;
            }
        }

        let callback = progress_callback.lock().await;
        callback(end_height);
        log::debug!("Witnesses {}", witnesses.len());

        Ok::<_, anyhow::Error>(())
    });

    let mut height = start_height;
    while height < end_height {
        let s = height;
        let e = (height + chunk_size).min(end_height);
        let range = s..e;

        let _ = downloader_tx.send(range).await;

        height = e;
    }
    drop(downloader_tx);
    log::info!("req downloading completed");

    let res = tokio::try_join!(downloader, processor);
    match res {
        Ok((d, p)) => {
            if let Err(err) = d {
                log::info!("Downloader error = {}", err);
                return Err(err);
            }
            if let Err(err) = p {
                log::info!("Processor error = {}", err);
                return Err(err);
            }
        }
        Err(err) => {
            log::info!("Sync error = {}", err);
            if err.is_panic() {
                panic::resume_unwind(err.into_panic());
            }
            anyhow::bail!("Join Error");
        },
    }

    log::info!("Sync completed");

    Ok(())
}

pub async fn latest_height(ld_url: &str) -> anyhow::Result<u32> {
    let mut client = connect_lightwalletd(ld_url).await?;
    let height = get_latest_height(&mut client).await?;
    Ok(height)
}
