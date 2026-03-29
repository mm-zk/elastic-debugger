use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::fs;
use std::path::Path;

use crate::addresses::{address_to_human, u256_to_address};
use crate::{sequencer::Sequencer, utils::get_all_events};
use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::providers::Provider;
use alloy::rpc::types::Log;
use alloy::sol;
use alloy::sol_types::SolEvent;
use colored::Colorize;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

sol! {
    struct L2CanonicalTransaction {
        uint256 txType;
        uint256 from;
        uint256 to;
        uint256 gasLimit;
        uint256 gasPerPubdataByteLimit;
        uint256 maxFeePerGas;
        uint256 maxPriorityFeePerGas;
        uint256 paymaster;
        uint256 nonce;
        uint256 value;
        // In the future, we might want to add some
        // new fields to the struct. The `txData` struct
        // is to be passed to account and any changes to its structure
        // would mean a breaking change to these accounts. To prevent this,
        // we should keep some fields as "reserved"
        // It is also recommended that their length is fixed, since
        // it would allow easier proof integration (in case we will need
        // some special circuit for preprocessing transactions)
        uint256[4] reserved;
        bytes data;
        bytes signature;
        uint256[] factoryDeps;
        bytes paymasterInput;
        // Reserved dynamic type for the future use-case. Using it should be avoided,
        // But it is still here, just in case we want to enable some additional functionality
        bytes reservedDynamic;
    }

    #[sol(rpc)]
    contract IMailbox {
        event NewPriorityRequest(
        uint256 txId,
        bytes32 txHash,
        uint64 expirationTimestamp,
        L2CanonicalTransaction transaction,
        bytes[] factoryDeps
    );
}
}

lazy_static! {
    static ref KNOWN_SIGNATURES: HashMap<String, String> = {
        let json_value = serde_json::from_slice(include_bytes!("data/abi_map.json")).unwrap();
        let pairs: HashMap<String, String> = serde_json::from_value(json_value).unwrap();

        pairs
    };
}

#[derive(Serialize)]
pub struct PriorityTransactionReport {
    pub index: u64,
    pub tx_id: String,
    pub expiration_timestamp: u64,
    pub from: String,
    pub to: String,
    pub value_wei: String,
    pub value_formatted: String,
    pub gas_limit: String,
    pub gas_per_pubdata_byte_limit: String,
    pub max_fee_per_gas: String,
    pub max_priority_fee_per_gas: String,
    pub method: Option<String>,
    pub data: String,
}

pub struct PriorityTransaction {
    pub index: u64,
    tx_id: B256,
    expiration_timestamp: u64,
    l2_tx: L2CanonicalTransaction,
}

impl Debug for PriorityTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriorityTransaction")
            .field("index", &self.index)
            .field("tx_id", &self.tx_id)
            .field("expiration_timestamp", &self.expiration_timestamp)
            .finish()
    }
}

impl Display for PriorityTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.detailed_fmt(f, 0)
    }
}

fn format_integer_with_underscores(input: &str) -> String {
    let reversed_input: String = input.chars().rev().collect();

    // Insert underscores every three characters
    let mut formatted = String::new();
    for (index, char) in reversed_input.chars().enumerate() {
        if index % 3 == 0 && index != 0 {
            formatted.push('_');
        }
        formatted.push(char);
    }

    // Reverse the formatted string to correct the order
    formatted.chars().rev().collect()
}

pub fn wei_as_string(value: U256) -> String {
    format_integer_with_underscores(&value.to_string())
}

impl PriorityTransaction {
    pub fn detailed_fmt(&self, f: &mut std::fmt::Formatter<'_>, pad: usize) -> std::fmt::Result {
        let pad = " ".repeat(pad);
        writeln!(f, "{}Tx: {} - {}", pad, self.index, self.tx_id)?;

        writeln!(
            f,
            "{}    {} -> {}",
            pad,
            address_to_human(&u256_to_address(self.l2_tx.from)),
            address_to_human(&u256_to_address(self.l2_tx.to))
        )?;

        if self.l2_tx.data.len() > 4 {
            let selector = hex::encode(&self.l2_tx.data[0..4]);
            let entry = KNOWN_SIGNATURES.get(&selector).unwrap_or(&selector);

            writeln!(f, "{}    Method           - {}", pad, entry.bold())?;
        }

        if self.l2_tx.reserved[0] > U256::ZERO {
            writeln!(
                f,
                "{}    Value (reserved) - {}",
                pad,
                wei_as_string(self.l2_tx.reserved[0])
            )?;
        }
        Ok(())
    }

    pub fn to_report(&self) -> PriorityTransactionReport {
        let method = if self.l2_tx.data.len() > 4 {
            let selector = hex::encode(&self.l2_tx.data[0..4]);
            Some(KNOWN_SIGNATURES.get(&selector).cloned().unwrap_or(selector))
        } else {
            None
        };

        PriorityTransactionReport {
            index: self.index,
            tx_id: format!("{:#x}", self.tx_id),
            expiration_timestamp: self.expiration_timestamp,
            from: address_to_human(&u256_to_address(self.l2_tx.from)),
            to: address_to_human(&u256_to_address(self.l2_tx.to)),
            value_wei: self.l2_tx.value.to_string(),
            value_formatted: wei_as_string(self.l2_tx.value),
            gas_limit: self.l2_tx.gasLimit.to_string(),
            gas_per_pubdata_byte_limit: self.l2_tx.gasPerPubdataByteLimit.to_string(),
            max_fee_per_gas: self.l2_tx.maxFeePerGas.to_string(),
            max_priority_fee_per_gas: self.l2_tx.maxPriorityFeePerGas.to_string(),
            method,
            data: format!("0x{}", hex::encode(&self.l2_tx.data)),
        }
    }
}

impl From<Log> for PriorityTransaction {
    fn from(value: Log) -> Self {
        let request =
            IMailbox::NewPriorityRequest::abi_decode_data(&value.data().data, true).unwrap();

        let index: u64 = request.0.try_into().unwrap();
        let tx_id = request.1;
        let expiration_timestamp = request.2;

        Self {
            index,
            tx_id,
            expiration_timestamp,
            l2_tx: request.3,
        }
    }
}

pub fn compute_merkle_tree(txs: &Vec<PriorityTransaction>) -> B256 {
    let size = txs.len().next_power_of_two();
    let mut leaves = vec![keccak256(""); size];
    for tx in txs {
        leaves[tx.index as usize] = tx.tx_id;
    }
    while leaves.len() > 1 {
        let mut parents = vec![];

        for i in 0..(leaves.len() / 2) {
            let payload = [leaves[2 * i].as_slice(), leaves[2 * i + 1].as_slice()].concat();

            parents.push(keccak256(payload));
        }
        leaves = parents;
    }

    *leaves.get(0).unwrap()
}

// Cache format for priority transaction logs
#[derive(Serialize, Deserialize)]
struct PriorityTxCacheFile {
    version: u32,
    chain_id: u64,
    hyperchain_address: String,
    /// The highest block number covered by cached logs.
    last_block: u64,
    txs: Vec<CachedPriorityTx>,
}

#[derive(Serialize, Deserialize)]
struct CachedPriorityTx {
    index: u64,
    tx_id: String,
    expiration_timestamp: u64,
    /// Raw ABI-encoded log data (hex) so we can re-decode the full L2CanonicalTransaction.
    raw_log_data: String,
}

const PRIORITY_TX_CACHE_VERSION: u32 = 1;

fn cache_path(cache_dir: &Path, chain_id: u64, address: Address) -> std::path::PathBuf {
    cache_dir.join(format!("priority_txs-{chain_id}-{address:#x}.json"))
}

fn load_cache(
    cache_dir: &Path,
    chain_id: u64,
    address: Address,
) -> Option<PriorityTxCacheFile> {
    let path = cache_path(cache_dir, chain_id, address);
    let contents = fs::read(&path).ok()?;
    let cache: PriorityTxCacheFile = serde_json::from_slice(&contents).ok()?;
    if cache.version != PRIORITY_TX_CACHE_VERSION
        || cache.chain_id != chain_id
        || cache.hyperchain_address != format!("{address:#x}")
    {
        return None;
    }
    Some(cache)
}

fn store_cache(
    cache_dir: &Path,
    chain_id: u64,
    address: Address,
    last_block: u64,
    txs: &[PriorityTransaction],
    raw_data: &HashMap<u64, Vec<u8>>,
) -> eyre::Result<()> {
    fs::create_dir_all(cache_dir)?;

    let cached_txs: Vec<CachedPriorityTx> = txs
        .iter()
        .map(|tx| CachedPriorityTx {
            index: tx.index,
            tx_id: format!("{:#x}", tx.tx_id),
            expiration_timestamp: tx.expiration_timestamp,
            raw_log_data: format!(
                "0x{}",
                hex::encode(raw_data.get(&tx.index).unwrap_or(&vec![]))
            ),
        })
        .collect();

    let cache = PriorityTxCacheFile {
        version: PRIORITY_TX_CACHE_VERSION,
        chain_id,
        hyperchain_address: format!("{address:#x}"),
        last_block,
        txs: cached_txs,
    };

    let path = cache_path(cache_dir, chain_id, address);
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, serde_json::to_vec_pretty(&cache)?)?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

fn decode_cached_txs(cached: &[CachedPriorityTx]) -> (Vec<PriorityTransaction>, HashMap<u64, Vec<u8>>) {
    let mut txs = Vec::new();
    let mut raw_data = HashMap::new();
    for ct in cached {
        let data_bytes = match hex::decode(ct.raw_log_data.strip_prefix("0x").unwrap_or(&ct.raw_log_data)) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let request = match IMailbox::NewPriorityRequest::abi_decode_data(&data_bytes, true) {
            Ok(r) => r,
            Err(_) => continue,
        };
        raw_data.insert(ct.index, data_bytes);
        txs.push(PriorityTransaction {
            index: ct.index,
            tx_id: request.1,
            expiration_timestamp: request.2,
            l2_tx: request.3,
        });
    }
    (txs, raw_data)
}

pub async fn fetch_all_priority_transactions(
    sequencer: &Sequencer,
    address: Address,
    cache_dir: &Path,
) -> eyre::Result<Vec<PriorityTransaction>> {
    match sequencer.sequencer_type {
        crate::sequencer::SequencerType::L1 => {
            let provider = sequencer.get_provider();
            let current_block = provider.get_block_number().await?;
            let block_limit: u64 = 5000;

            // Load cached txs if available
            let (mut all_txs, mut raw_data, fetch_from_block) =
                match load_cache(cache_dir, sequencer.chain_id, address) {
                    Some(cache) => {
                        let (txs, raw) = decode_cached_txs(&cache.txs);
                        let from = cache.last_block + 1;
                        println!(
                            "  Loaded {} cached priority txs (up to block {})",
                            txs.len(),
                            cache.last_block
                        );
                        (txs, raw, from)
                    }
                    None => {
                        let from = current_block.saturating_sub(block_limit);
                        (vec![], HashMap::new(), from)
                    }
                };

            // Fetch only new logs from where the cache left off
            if fetch_from_block <= current_block {
                let remaining = current_block - fetch_from_block + 1;
                let new_logs = get_all_events(
                    sequencer,
                    address,
                    IMailbox::NewPriorityRequest::SIGNATURE_HASH,
                    remaining,
                    Some(fetch_from_block),
                )
                .await?;

                if !new_logs.is_empty() {
                    println!("  Fetched {} new priority tx logs", new_logs.len());
                }

                for log in new_logs {
                    let log_data_bytes = log.data().data.to_vec();
                    let tx = PriorityTransaction::from(log);
                    raw_data.insert(tx.index, log_data_bytes);
                    all_txs.push(tx);
                }
            }

            // Deduplicate by index (in case of overlap)
            let mut seen: HashMap<u64, PriorityTransaction> = HashMap::new();
            for tx in all_txs {
                seen.entry(tx.index).or_insert(tx);
            }
            let txs: Vec<PriorityTransaction> = seen.into_values().collect();

            // Store for next time
            store_cache(
                cache_dir,
                sequencer.chain_id,
                address,
                current_block,
                &txs,
                &raw_data,
            )?;

            Ok(txs)
        }
        crate::sequencer::SequencerType::L2(_) => {
            eyre::bail!("Priority transactions are only available on L1");
        }
    }
}
