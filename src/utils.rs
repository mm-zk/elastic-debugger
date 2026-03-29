use alloy::{
    primitives::{keccak256, Address, B256},
    providers::Provider,
    rpc::types::{Filter, Log},
};
use names::{ADJECTIVES, NOUNS};

use crate::sequencer::Sequencer;

pub async fn get_all_events(
    sequencer: &Sequencer,
    address: Address,
    signature: B256,
    block_limit: u64,
    start_from_block: Option<u64>,
) -> eyre::Result<Vec<Log>> {
    let provider = sequencer.get_provider();
    let mut current_block = provider.get_block_number().await?;
    let mut result = vec![];
    let mut blocks_per_call: u64 = 500;

    let min_block = match start_from_block {
        Some(from) => from,
        None => current_block.saturating_sub(block_limit),
    };

    while current_block >= min_block && current_block > 0 {
        let from_block = current_block.saturating_sub(blocks_per_call).max(min_block);

        let filter = Filter::new()
            .from_block(from_block)
            .to_block(current_block)
            .event_signature(signature)
            .address(address);

        match sequencer.get_provider().get_logs(&filter).await {
            Ok(mut logs) => {
                result.append(&mut logs);
                if from_block == 0 {
                    break;
                }
                current_block = from_block - 1;
            }
            Err(e) => {
                let err_str = e.to_string();
                // If the provider limits block range, reduce our chunk size and retry
                if err_str.contains("block range") || err_str.contains("-32600") {
                    blocks_per_call = (blocks_per_call / 10).max(10);
                    continue;
                }
                // Rate limit - wait and retry
                if err_str.contains("429") {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
                return Err(e.into());
            }
        }
    }

    Ok(result)
}

pub fn get_human_name_for<T: AsRef<[u8]>>(entry: T) -> String {
    let hashed_address = keccak256(entry);
    let pos = usize::from_be_bytes(hashed_address[0..8].try_into().unwrap());
    format!(
        "{}_{}",
        ADJECTIVES[pos % ADJECTIVES.len()],
        NOUNS[pos % NOUNS.len()]
    )
}

/*pub fn address_from_fixedbytes(bytes: &FixedBytes<32>) -> eyre::Result<Address> {
    for i in 0..12 {
        if bytes.0[i] != 0 {
            eyre::bail!("cannot cast 32 bytes to address - non zero value in first 12 bytes");
        }
    }

    Ok(Address::from_slice(&bytes.0[12..32]))
}*/
