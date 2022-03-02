use crate::chain::{Nf, NfRef};
use crate::contact::Contact;
use crate::prices::Quote;
use crate::taddr::derive_tkeys;
use crate::transaction::TransactionInfo;
use crate::{CTree, Witness};
use chrono::NaiveDateTime;
use rusqlite::{params, Connection, OptionalExtension, Transaction, NO_PARAMS};
use std::collections::HashMap;
use bech32::FromBase32;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::consensus::{Network, NetworkUpgrade, Parameters};
use zcash_primitives::merkle_tree::IncrementalWitness;
use zcash_primitives::sapling::{Diversifier, Node, Note, Rseed, SaplingIvk};
use zcash_primitives::zip32::{DiversifierIndex, ExtendedFullViewingKey};
use serde::{Serialize, Deserialize};
use chacha20poly1305::{Key, ChaCha20Poly1305, Nonce};
use chacha20poly1305::aead::{Aead, NewAead};
use zcash_params::coin::{CoinType, get_coin_chain};

mod migration;

#[allow(dead_code)]
pub const DEFAULT_DB_PATH: &str = "zec.db";

pub struct DbAdapter {
    pub coin_type: CoinType,
    pub connection: Connection,
}

pub struct ReceivedNote {
    pub account: u32,
    pub height: u32,
    pub output_index: u32,
    pub diversifier: Vec<u8>,
    pub value: u64,
    pub rcm: Vec<u8>,
    pub nf: Vec<u8>,
    pub spent: Option<u32>,
}

#[derive(Clone)]
pub struct SpendableNote {
    pub id: u32,
    pub note: Note,
    pub diversifier: Diversifier,
    pub witness: IncrementalWitness<Node>,
}

pub struct AccountViewKey {
    pub fvk: ExtendedFullViewingKey,
    pub ivk: SaplingIvk,
    pub viewonly: bool,
}

impl AccountViewKey {
    pub fn from_fvk(fvk: &ExtendedFullViewingKey) -> AccountViewKey {
        AccountViewKey {
            fvk: fvk.clone(),
            ivk: fvk.fvk.vk.ivk(),
            viewonly: false,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AccountBackup {
    pub name: String,
    pub seed: Option<String>,
    pub z_sk: Option<String>,
    pub ivk: String,
    pub z_addr: String,
    pub t_sk: Option<String>,
    pub t_addr: Option<String>,
}

impl DbAdapter {
    pub fn new(coin_type: CoinType, db_path: &str) -> anyhow::Result<DbAdapter> {
        let connection = Connection::open(db_path)?;
        log::info!("{:?} {}", coin_type, db_path);
        connection.execute("PRAGMA synchronous = off", NO_PARAMS)?;
        Ok(DbAdapter { coin_type, connection })
    }

    pub fn begin_transaction(&mut self) -> anyhow::Result<Transaction> {
        let tx = self.connection.transaction()?;
        Ok(tx)
    }
    //
    // pub fn commit(&self) -> anyhow::Result<()> {
    //     self.connection.execute("COMMIT", NO_PARAMS)?;
    //     Ok(())
    // }
    //
    pub fn init_db(&self) -> anyhow::Result<()> {
        migration::init_db(&self.connection)?;
        Ok(())
    }

    pub fn reset_db(&self) -> anyhow::Result<()> {
        migration::reset_db(&self.connection)?;
        Ok(())
    }

    pub fn store_account(
        &self,
        name: &str,
        seed: Option<&str>,
        sk: Option<&str>,
        ivk: &str,
        address: &str,
    ) -> anyhow::Result<i32> {
        let mut statement = self
            .connection
            .prepare("SELECT id_account FROM accounts WHERE ivk = ?1")?;
        if statement.exists(params![ivk])? {
            return Ok(-1);
        }
        self.connection.execute(
            "INSERT INTO accounts(name, seed, sk, ivk, address) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT DO NOTHING",
            params![name, seed, sk, ivk, address],
        )?;
        let id_tx: i32 = self.connection.query_row(
            "SELECT id_account FROM accounts WHERE ivk = ?1",
            params![ivk],
            |row| row.get(0),
        )?;
        Ok(id_tx)
    }

    pub fn get_fvks(&self) -> anyhow::Result<HashMap<u32, AccountViewKey>> {
        let mut statement = self
            .connection
            .prepare("SELECT id_account, ivk, sk FROM accounts")?;
        let rows = statement.query_map(NO_PARAMS, |row| {
            let account: u32 = row.get(0)?;
            let ivk: String = row.get(1)?;
            let sk: Option<String> = row.get(2)?;
            let fvk = decode_extended_full_viewing_key(
                self.network().hrp_sapling_extended_full_viewing_key(),
                &ivk,
            )
            .unwrap()
            .unwrap();
            let ivk = fvk.fvk.vk.ivk();
            Ok((
                account,
                AccountViewKey {
                    fvk,
                    ivk,
                    viewonly: sk.is_none(),
                },
            ))
        })?;
        let mut fvks: HashMap<u32, AccountViewKey> = HashMap::new();
        for r in rows {
            let row = r?;
            fvks.insert(row.0, row.1);
        }
        Ok(fvks)
    }

    pub fn trim_to_height(&mut self, height: u32) -> anyhow::Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM blocks WHERE height >= ?1", params![height])?;
        tx.execute(
            "DELETE FROM sapling_witnesses WHERE height >= ?1",
            params![height],
        )?;
        tx.execute(
            "DELETE FROM received_notes WHERE height >= ?1",
            params![height],
        )?;
        tx.execute(
            "UPDATE received_notes SET spent = NULL WHERE spent >= ?1",
            params![height],
        )?;
        tx.execute(
            "DELETE FROM transactions WHERE height >= ?1",
            params![height],
        )?;
        tx.commit()?;

        Ok(())
    }

    pub fn get_txhash(&self, id_tx: u32) -> anyhow::Result<(u32, u32, Vec<u8>, String)> {
        let (account, height, tx_hash, ivk) = self.connection.query_row(
            "SELECT account, height, txid, ivk FROM transactions t, accounts a WHERE id_tx = ?1 AND t.account = a.id_account",
            params![id_tx],
            |row| {
                let account: u32 = row.get(0)?;
                let height: u32 = row.get(1)?;
                let tx_hash: Vec<u8> = row.get(2)?;
                let ivk: String = row.get(3)?;
                Ok((account, height, tx_hash, ivk))
            },
        )?;
        Ok((account, height, tx_hash, ivk))
    }

    pub fn store_block(
        &self,
        height: u32,
        hash: &[u8],
        timestamp: u32,
        tree: &CTree,
    ) -> anyhow::Result<()> {
        log::debug!("+block");
        let mut bb: Vec<u8> = vec![];
        tree.write(&mut bb)?;
        self.connection.execute(
            "INSERT INTO blocks(height, hash, timestamp, sapling_tree)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT DO NOTHING",
            params![height, hash, timestamp, &bb],
        )?;
        log::debug!("-block");
        Ok(())
    }

    pub fn store_transaction(
        txid: &[u8],
        account: u32,
        height: u32,
        timestamp: u32,
        tx_index: u32,
        db_tx: &Transaction,
    ) -> anyhow::Result<u32> {
        log::debug!("+transaction");
        db_tx.execute(
            "INSERT INTO transactions(account, txid, height, timestamp, tx_index, value)
        VALUES (?1, ?2, ?3, ?4, ?5, 0)
        ON CONFLICT DO NOTHING",
            params![account, txid, height, timestamp, tx_index],
        )?;
        let id_tx: u32 = db_tx.query_row(
            "SELECT id_tx FROM transactions WHERE account = ?1 AND txid = ?2",
            params![account, txid],
            |row| row.get(0),
        )?;
        log::debug!("-transaction {}", id_tx);
        Ok(id_tx)
    }

    pub fn store_received_note(
        note: &ReceivedNote,
        id_tx: u32,
        position: usize,
        db_tx: &Transaction,
    ) -> anyhow::Result<u32> {
        log::debug!("+received_note {}", id_tx);
        db_tx.execute("INSERT INTO received_notes(account, tx, height, position, output_index, diversifier, value, rcm, nf, spent)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT DO NOTHING", params![note.account, id_tx, note.height, position as u32, note.output_index, note.diversifier, note.value as i64, note.rcm, note.nf, note.spent])?;
        let id_note: u32 = db_tx.query_row(
            "SELECT id_note FROM received_notes WHERE tx = ?1 AND output_index = ?2",
            params![id_tx, note.output_index],
            |row| row.get(0),
        )?;
        log::debug!("-received_note");
        Ok(id_note)
    }

    pub fn store_witnesses(
        &self,
        witness: &Witness,
        height: u32,
        id_note: u32,
    ) -> anyhow::Result<()> {
        log::debug!("+witnesses");
        let mut bb: Vec<u8> = vec![];
        witness.write(&mut bb)?;
        self.connection.execute(
            "INSERT INTO sapling_witnesses(note, height, witness) VALUES (?1, ?2, ?3)
        ON CONFLICT DO NOTHING",
            params![id_note, height, bb],
        )?;
        log::debug!("-witnesses");
        Ok(())
    }

    pub fn store_tx_metadata(&self, id_tx: u32, tx_info: &TransactionInfo) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE transactions SET address = ?1, memo = ?2 WHERE id_tx = ?3",
            params![tx_info.address, &tx_info.memo, id_tx],
        )?;
        Ok(())
    }

    pub fn add_value(id_tx: u32, value: i64, db_tx: &Transaction) -> anyhow::Result<()> {
        db_tx.execute(
            "UPDATE transactions SET value = value + ?2 WHERE id_tx = ?1",
            params![id_tx, value],
        )?;
        Ok(())
    }

    pub fn get_received_note_value(nf: &Nf, db_tx: &Transaction) -> anyhow::Result<(u32, i64)> {
        let (account, value) = db_tx.query_row(
            "SELECT account, value FROM received_notes WHERE nf = ?1",
            params![nf.0.to_vec()],
            |row| {
                let account: u32 = row.get(0)?;
                let value: i64 = row.get(1)?;
                Ok((account, value))
            },
        )?;
        Ok((account, value))
    }

    pub fn get_balance(&self, account: u32) -> anyhow::Result<u64> {
        let balance: Option<i64> = self.connection.query_row(
            "SELECT SUM(value) FROM received_notes WHERE (spent IS NULL OR spent = 0) AND account = ?1",
            params![account],
            |row| row.get(0),
        )?;
        Ok(balance.unwrap_or(0) as u64)
    }

    pub fn get_last_sync_height(&self) -> anyhow::Result<Option<u32>> {
        let height: Option<u32> =
            self.connection
                .query_row("SELECT MAX(height) FROM blocks", NO_PARAMS, |row| {
                    row.get(0)
                })?;
        Ok(height)
    }

    pub fn get_db_height(&self) -> anyhow::Result<u32> {
        let height: u32 = self.get_last_sync_height()?.unwrap_or_else(|| {
            self.network()
                .activation_height(NetworkUpgrade::Sapling)
                .unwrap()
                .into()
        });
        Ok(height)
    }

    pub fn get_db_hash(&self, height: u32) -> anyhow::Result<Option<[u8; 32]>> {
        let hash: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT hash FROM blocks WHERE height = ?1",
                params![height],
                |row| row.get(0),
            )
            .optional()?;
        Ok(hash.map(|h| {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&h);
            hash
        }))
    }

    pub fn get_tree(&self) -> anyhow::Result<(CTree, Vec<Witness>)> {
        let res = self.connection.query_row(
            "SELECT height, sapling_tree FROM blocks WHERE height = (SELECT MAX(height) FROM blocks)",
            NO_PARAMS, |row| {
                let height: u32 = row.get(0)?;
                let tree: Vec<u8> = row.get(1)?;
                Ok((height, tree))
            }).optional()?;
        Ok(match res {
            Some((height, tree)) => {
                let tree = CTree::read(&*tree)?;
                let mut statement = self.connection.prepare(
                    "SELECT id_note, witness FROM sapling_witnesses w, received_notes n WHERE w.height = ?1 AND w.note = n.id_note AND (n.spent IS NULL OR n.spent = 0)")?;
                let ws = statement.query_map(params![height], |row| {
                    let id_note: u32 = row.get(0)?;
                    let witness: Vec<u8> = row.get(1)?;
                    Ok(Witness::read(id_note, &*witness).unwrap())
                })?;
                let mut witnesses: Vec<Witness> = vec![];
                for w in ws {
                    witnesses.push(w?);
                }
                (tree, witnesses)
            }
            None => (CTree::new(), vec![]),
        })
    }

    pub fn get_nullifiers(&self) -> anyhow::Result<HashMap<Nf, NfRef>> {
        let mut statement = self.connection.prepare(
            "SELECT id_note, account, nf FROM received_notes WHERE spent IS NULL OR spent = 0",
        )?;
        let nfs_res = statement.query_map(NO_PARAMS, |row| {
            let id_note: u32 = row.get(0)?;
            let account: u32 = row.get(1)?;
            let nf_vec: Vec<u8> = row.get(2)?;
            let mut nf = [0u8; 32];
            nf.clone_from_slice(&nf_vec);
            let nf_ref = NfRef { id_note, account };
            Ok((nf_ref, nf))
        })?;
        let mut nfs: HashMap<Nf, NfRef> = HashMap::new();
        for n in nfs_res {
            let n = n?;
            nfs.insert(Nf(n.1), n.0);
        }

        Ok(nfs)
    }

    pub fn get_nullifier_amounts(
        &self,
        account: u32,
        unspent_only: bool,
    ) -> anyhow::Result<HashMap<Vec<u8>, u64>> {
        let mut sql = "SELECT value, nf FROM received_notes WHERE account = ?1".to_string();
        if unspent_only {
            sql += "AND (spent IS NULL OR spent = 0)";
        }
        let mut statement = self.connection.prepare(&sql)?;
        let nfs_res = statement.query_map(params![account], |row| {
            let amount: i64 = row.get(0)?;
            let nf: Vec<u8> = row.get(1)?;
            Ok((amount, nf))
        })?;
        let mut nfs: HashMap<Vec<u8>, u64> = HashMap::new();
        for n in nfs_res {
            let n = n?;
            nfs.insert(n.1, n.0 as u64);
        }

        Ok(nfs)
    }

    pub fn get_nullifiers_raw(&self) -> anyhow::Result<Vec<(u32, u64, Vec<u8>)>> {
        let mut statement = self
            .connection
            .prepare("SELECT account, value, nf FROM received_notes")?;
        let res = statement.query_map(NO_PARAMS, |row| {
            let account: u32 = row.get(0)?;
            let amount: i64 = row.get(1)?;
            let nf: Vec<u8> = row.get(2)?;
            Ok((account, amount as u64, nf))
        })?;
        let mut v: Vec<(u32, u64, Vec<u8>)> = vec![];
        for r in res {
            v.push(r?);
        }
        Ok(v)
    }

    pub fn get_spendable_notes(
        &self,
        account: u32,
        anchor_height: u32,
        fvk: &ExtendedFullViewingKey,
    ) -> anyhow::Result<Vec<SpendableNote>> {
        let mut statement = self.connection.prepare(
            "SELECT id_note, diversifier, value, rcm, witness FROM received_notes r, sapling_witnesses w WHERE spent IS NULL AND account = ?2
            AND (r.excluded IS NULL OR NOT r.excluded) AND w.height = (
	            SELECT MAX(height) FROM sapling_witnesses WHERE height <= ?1
            ) AND r.id_note = w.note")?;
        let notes = statement.query_map(params![anchor_height, account], |row| {
            let id_note: u32 = row.get(0)?;

            let diversifier: Vec<u8> = row.get(1)?;
            let value: i64 = row.get(2)?;
            let rcm: Vec<u8> = row.get(3)?;
            let witness: Vec<u8> = row.get(4)?;

            let mut diversifer_bytes = [0u8; 11];
            diversifer_bytes.copy_from_slice(&diversifier);
            let diversifier = Diversifier(diversifer_bytes);
            let mut rcm_bytes = [0u8; 32];
            rcm_bytes.copy_from_slice(&rcm);
            let rcm = jubjub::Fr::from_bytes(&rcm_bytes).unwrap();
            let rseed = Rseed::BeforeZip212(rcm);
            let witness = IncrementalWitness::<Node>::read(&*witness).unwrap();

            let pa = fvk.fvk.vk.to_payment_address(diversifier).unwrap();
            let note = pa.create_note(value as u64, rseed).unwrap();
            Ok(SpendableNote {
                id: id_note,
                note,
                diversifier,
                witness,
            })
        })?;
        let mut spendable_notes: Vec<SpendableNote> = vec![];
        for n in notes {
            spendable_notes.push(n?);
        }

        Ok(spendable_notes)
    }

    pub fn mark_spent(id: u32, height: u32, tx: &Transaction) -> anyhow::Result<()> {
        log::debug!("+mark_spent");
        tx.execute(
            "UPDATE received_notes SET spent = ?1 WHERE id_note = ?2",
            params![height, id],
        )?;
        log::debug!("-mark_spent");
        Ok(())
    }

    pub fn purge_old_witnesses(&self, height: u32) -> anyhow::Result<()> {
        log::debug!("+purge_old_witnesses");
        let min_height: Option<u32> = self.connection.query_row(
            "SELECT MAX(height) FROM sapling_witnesses WHERE height <= ?1",
            params![height],
            |row| row.get(0),
        )?;

        // Leave at least one sapling witness
        if let Some(min_height) = min_height {
            log::debug!("Purging witnesses older than {}", min_height);
            self.connection.execute(
                "DELETE FROM sapling_witnesses WHERE height < ?1",
                params![min_height],
            )?;
            self.connection
                .execute("DELETE FROM blocks WHERE height < ?1", params![min_height])?;
        }
        log::debug!("-purge_old_witnesses");
        Ok(())
    }

    pub fn store_contact(&self, contact: &Contact, dirty: bool) -> anyhow::Result<()> {
        if contact.id == 0 {
            self.connection.execute(
                "INSERT INTO contacts(name, address, dirty)
                VALUES (?1, ?2, ?3)",
                params![&contact.name, &contact.address, dirty],
            )?;
        } else {
            self.connection.execute(
                "INSERT INTO contacts(id, name, address, dirty)
                VALUES (?1, ?2, ?3, ?4) ON CONFLICT (id) DO UPDATE SET
                name = excluded.name, address = excluded.address, dirty = excluded.dirty",
                params![contact.id, &contact.name, &contact.address, dirty],
            )?;
        }
        Ok(())
    }

    pub fn get_unsaved_contacts(&self) -> anyhow::Result<Vec<Contact>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, address FROM contacts WHERE dirty = TRUE")?;
        let rows = statement.query_map(NO_PARAMS, |row| {
            let id: u32 = row.get(0)?;
            let name: String = row.get(1)?;
            let address: String = row.get(2)?;
            let contact = Contact { id, name, address };
            Ok(contact)
        })?;
        let mut contacts: Vec<Contact> = vec![];
        for r in rows {
            contacts.push(r?);
        }

        Ok(contacts)
    }

    pub fn get_backup(
        &self,
        account: u32,
    ) -> anyhow::Result<(Option<String>, Option<String>, String)> {
        log::debug!("+get_backup");
        let (seed, sk, ivk) = self.connection.query_row(
            "SELECT seed, sk, ivk FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let seed: Option<String> = row.get(0)?;
                let sk: Option<String> = row.get(0)?;
                let ivk: String = row.get(0)?;
                Ok((seed, sk, ivk))
            },
        )?;
        log::debug!("-get_backup");
        Ok((seed, sk, ivk))
    }

    pub fn get_seed(&self, account: u32) -> anyhow::Result<Option<String>> {
        log::info!("+get_seed");
        let seed = self.connection.query_row(
            "SELECT seed FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let sk: Option<String> = row.get(0)?;
                Ok(sk)
            },
        )?;
        log::info!("-get_seed");
        Ok(seed)
    }

    pub fn get_sk(&self, account: u32) -> anyhow::Result<String> {
        log::info!("+get_sk");
        let sk = self.connection.query_row(
            "SELECT sk FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let sk: String = row.get(0)?;
                Ok(sk)
            },
        )?;
        log::info!("-get_sk");
        Ok(sk)
    }

    pub fn get_ivk(&self, account: u32) -> anyhow::Result<String> {
        log::debug!("+get_ivk");
        let ivk = self.connection.query_row(
            "SELECT ivk FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let ivk: String = row.get(0)?;
                Ok(ivk)
            },
        )?;
        log::debug!("-get_ivk");
        Ok(ivk)
    }

    pub fn get_address(&self, account: u32) -> anyhow::Result<String> {
        log::debug!("+get_address");
        let address = self.connection.query_row(
            "SELECT address FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let address: String = row.get(0)?;
                Ok(address)
            },
        )?;
        log::debug!("-get_address");
        Ok(address)
    }

    pub fn get_diversifier(&self, account: u32) -> anyhow::Result<DiversifierIndex> {
        let diversifier_index = self
            .connection
            .query_row(
                "SELECT diversifier_index FROM diversifiers WHERE account = ?1",
                params![account],
                |row| {
                    let d: Vec<u8> = row.get(0)?;
                    let mut div = [0u8; 11];
                    div.copy_from_slice(&d);
                    Ok(div)
                },
            )
            .optional()?
            .unwrap_or_else(|| [0u8; 11]);
        Ok(DiversifierIndex(diversifier_index))
    }

    pub fn store_diversifier(
        &self,
        account: u32,
        diversifier_index: &DiversifierIndex,
    ) -> anyhow::Result<()> {
        let diversifier_bytes = diversifier_index.0.to_vec();
        self.connection.execute(
            "INSERT INTO diversifiers(account, diversifier_index) VALUES (?1, ?2) ON CONFLICT \
            (account) DO UPDATE SET diversifier_index = excluded.diversifier_index",
            params![account, diversifier_bytes],
        )?;
        Ok(())
    }

    pub fn get_taddr(&self, account: u32) -> anyhow::Result<Option<String>> {
        let address = self
            .connection
            .query_row(
                "SELECT address FROM taddrs WHERE account = ?1",
                params![account],
                |row| {
                    let address: String = row.get(0)?;
                    Ok(address)
                },
            )
            .optional()?;
        Ok(address)
    }

    pub fn get_tsk(&self, account: u32) -> anyhow::Result<Option<String>> {
        let sk = self
            .connection
            .query_row(
                "SELECT sk FROM taddrs WHERE account = ?1",
                params![account],
                |row| {
                    let address: String = row.get(0)?;
                    Ok(address)
                },
            )
            .optional()?;
        Ok(sk)
    }

    pub fn create_taddr(&self, account: u32) -> anyhow::Result<()> {
        let seed = self.get_seed(account)?;
        if let Some(seed) = seed {
            let bip44_path = format!("m/44'/{}'/0'/0/0", self.network().coin_type());
            let (sk, address) = derive_tkeys(self.network(), &seed, &bip44_path)?;
            self.connection.execute(
                "INSERT INTO taddrs(account, sk, address) VALUES (?1, ?2, ?3) \
            ON CONFLICT DO NOTHING",
                params![account, &sk, &address],
            )?;
        }
        Ok(())
    }

    pub fn get_missing_prices_timestamp(&self, currency: &str) -> anyhow::Result<Vec<i64>> {
        let mut statement = self.connection.prepare(
            "WITH t AS (SELECT timestamp, timestamp/86400 AS day FROM transactions), p AS (SELECT price, timestamp/86400 AS day FROM historical_prices WHERE currency = ?1) \
                SELECT t.timestamp FROM t LEFT JOIN p ON t.day = p.day WHERE p.price IS NULL")?;
        let res = statement.query_map(params![currency], |row| {
            let timestamp: i64 = row.get(0)?;
            Ok(timestamp)
        })?;
        let mut timestamps: Vec<i64> = vec![];
        for ts in res {
            let ts = NaiveDateTime::from_timestamp(ts?, 0);
            let ts_date = ts.date().and_hms(0, 0, 0); // at midnight
            timestamps.push(ts_date.timestamp());
        }
        timestamps.sort();
        timestamps.dedup();
        Ok(timestamps)
    }

    pub fn store_historical_prices(
        &mut self,
        prices: &[Quote],
        currency: &str,
    ) -> anyhow::Result<()> {
        let db_transaction = self.connection.transaction()?;
        {
            let mut statement = db_transaction.prepare(
                "INSERT INTO historical_prices(timestamp, price, currency) VALUES (?1, ?2, ?3)",
            )?;
            for q in prices {
                statement.execute(params![q.timestamp, q.price, currency])?;
            }
        }
        db_transaction.commit()?;
        Ok(())
    }

    pub fn get_latest_quote(&self, currency: &str) -> anyhow::Result<Option<Quote>> {
        let quote = self.connection.query_row(
            "SELECT timestamp, price FROM historical_prices WHERE currency = ?1 ORDER BY timestamp DESC",
            params![currency],
            |row| {
                let timestamp: i64 = row.get(0)?;
                let price: f64 = row.get(1)?;
                Ok(Quote { timestamp, price })
            }).optional()?;
        Ok(quote)
    }

    pub fn store_share_secret(
        &self,
        account: u32,
        secret: &str,
        index: usize,
        threshold: usize,
        participants: usize,
    ) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO secret_shares(account, secret, idx, threshold, participants) VALUES (?1, ?2, ?3, ?4, ?5) \
            ON CONFLICT (account) DO UPDATE SET secret = excluded.secret, threshold = excluded.threshold, participants = excluded.participants",
            params![account, &secret, index as u32, threshold as u32, participants as u32],
        )?;
        Ok(())
    }

    pub fn get_share_secret(&self, account: u32) -> anyhow::Result<String> {
        let secret = self
            .connection
            .query_row(
                "SELECT secret FROM secret_shares WHERE account = ?1",
                params![account],
                |row| {
                    let secret: String = row.get(0)?;
                    Ok(secret)
                },
            )
            .optional()?;
        Ok(secret.unwrap_or("".to_string()))
    }

    pub fn truncate_data(&self) -> anyhow::Result<()> {
        self.connection.execute("DELETE FROM blocks", NO_PARAMS)?;
        self.connection.execute("DELETE FROM contacts", NO_PARAMS)?;
        self.connection
            .execute("DELETE FROM diversifiers", NO_PARAMS)?;
        self.connection
            .execute("DELETE FROM historical_prices", NO_PARAMS)?;
        self.connection
            .execute("DELETE FROM received_notes", NO_PARAMS)?;
        self.connection
            .execute("DELETE FROM sapling_witnesses", NO_PARAMS)?;
        self.connection
            .execute("DELETE FROM transactions", NO_PARAMS)?;
        Ok(())
    }

    pub fn delete_account(&self, account: u32) -> anyhow::Result<()> {
        self.connection.execute(
            "DELETE FROM received_notes WHERE account = ?1",
            params![account],
        )?;
        self.connection.execute(
            "DELETE FROM transactions WHERE account = ?1",
            params![account],
        )?;
        self.connection.execute(
            "DELETE FROM diversifiers WHERE account = ?1",
            params![account],
        )?;
        self.connection.execute(
            "DELETE FROM accounts WHERE id_account = ?1",
            params![account],
        )?;
        self.connection
            .execute("DELETE FROM taddrs WHERE account = ?1", params![account])?;
        self.connection.execute(
            "DELETE FROM secret_shares WHERE account = ?1",
            params![account],
        )?;
        Ok(())
    }

    const NONCE: &'static[u8; 12] = b"unique nonce";

    pub fn get_full_backup(&self, key: &str) -> anyhow::Result<String> {
        let mut statement = self.connection.prepare(
            "SELECT name, seed, a.sk AS z_sk, ivk, a.address AS z_addr, t.sk as t_sk, t.address AS t_addr FROM accounts a LEFT JOIN taddrs t ON a.id_account = t.account")?;
        let rows = statement.query_map(NO_PARAMS, |r| {
            let name: String = r.get(0)?;
            let seed: Option<String> = r.get(1)?;
            let z_sk: Option<String> = r.get(2)?;
            let ivk: String = r.get(3)?;
            let z_addr: String = r.get(4)?;
            let t_sk: Option<String> = r.get(5)?;
            let t_addr: Option<String> = r.get(6)?;
            Ok(AccountBackup {
                name,
                seed,
                z_sk,
                ivk,
                z_addr,
                t_sk,
                t_addr,
            })
        })?;
        let mut accounts: Vec<AccountBackup> = vec![];
        for r in rows {
            accounts.push(r?);
        }
        let accounts_bin = bincode::serialize(&accounts)?;

        let (hrp, key, _) = bech32::decode(key)?;
        if hrp != "zwk" { anyhow::bail!("Invalid backup key") }
        let key = Vec::<u8>::from_base32(&key)?;
        let key = Key::from_slice(&key);

        let cipher = ChaCha20Poly1305::new(key);
        // nonce is constant because we always use a different key!
        let cipher_text = cipher.encrypt(Nonce::from_slice(Self::NONCE), &*accounts_bin).map_err(|_e| anyhow::anyhow!("Failed to encrypt backup"))?;
        let backup = base64::encode(cipher_text);
        Ok(backup)
    }

    pub fn restore_full_backup(&self, key: &str, backup: &str) -> anyhow::Result<()> {
        let (hrp, key, _) = bech32::decode(key)?;
        if hrp != "zwk" { anyhow::bail!("Not a valid decryption key"); }
        let key = Vec::<u8>::from_base32(&key)?;
        let key = Key::from_slice(&key);

        let cipher = ChaCha20Poly1305::new(key);
        let backup = base64::decode(backup)?;
        let backup = cipher.decrypt(Nonce::from_slice(Self::NONCE), &*backup).map_err(|_e| anyhow::anyhow!("Failed to decrypt backup"))?;

        let accounts: Vec<AccountBackup> = bincode::deserialize(&backup)?;
        for a in accounts {
            log::info!("{}", a.name);
            let do_insert = || {
                self.connection.execute("INSERT INTO accounts(name, seed, sk, ivk, address) VALUES (?1,?2,?3,?4,?5)",
                                        params![a.name, a.seed, a.z_sk, a.ivk, a.z_addr])?;
                let id_account = self.connection.last_insert_rowid() as u32;
                if let Some(t_addr) = a.t_addr {
                    self.connection.execute("INSERT INTO taddrs(account, sk, address) VALUES (?1,?2,?3)",
                                            params![id_account, a.t_sk, t_addr])?;
                }
                Ok::<_, anyhow::Error>(())
            };
            let _ = do_insert();
        }

        Ok(())
    }

    fn network(&self) -> &'static Network {
        let chain = get_coin_chain(self.coin_type);
        chain.network()
    }
}

#[cfg(test)]
mod tests {
    use bech32::{ToBase32, Variant};
    use zcash_params::coin::CoinType;
    use crate::db::{DbAdapter, ReceivedNote, DEFAULT_DB_PATH};
    use crate::{CTree, Witness};

    #[test]
    fn test_db_backup() {
        let db = DbAdapter::new(CoinType::Zcash, DEFAULT_DB_PATH).unwrap();
        let k = [0u8; 32];
        let k = bech32::encode("zwk", k.to_base32(), Variant::Bech32).unwrap();
        let b = db.get_full_backup(&k).unwrap();
        println!("{} {}", k, b);

        db.restore_full_backup(&k, &b).unwrap();
    }

    #[test]
    fn test_db() {
        let mut db = DbAdapter::new(CoinType::Zcash, DEFAULT_DB_PATH).unwrap();
        db.init_db().unwrap();
        db.trim_to_height(0).unwrap();

        db.store_block(1, &[0u8; 32], 0, &CTree::new()).unwrap();
        let db_tx = db.begin_transaction().unwrap();
        let id_tx = DbAdapter::store_transaction(&[0; 32], 1, 1, 0, 20, &db_tx).unwrap();
        DbAdapter::store_received_note(
            &ReceivedNote {
                account: 1,
                height: 1,
                output_index: 0,
                diversifier: vec![],
                value: 0,
                rcm: vec![],
                nf: vec![],
                spent: None,
            },
            id_tx,
            5,
            &db_tx,
        )
        .unwrap();
        let witness = Witness {
            position: 10,
            id_note: 0,
            note: None,
            tree: CTree::new(),
            filled: vec![],
            cursor: CTree::new(),
        };
        db_tx.commit().unwrap();
        db.store_witnesses(&witness, 1000, 1).unwrap();
    }

    #[test]
    fn test_balance() {
        let db = DbAdapter::new(CoinType::Zcash, DEFAULT_DB_PATH).unwrap();
        let balance = db.get_balance(1).unwrap();
        println!("{}", balance);
    }
}
