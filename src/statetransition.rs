use std::fmt::Display;

use alloy::primitives::{Address, U256};
use alloy::primitives::{FixedBytes, B256};
use alloy::sol;
use colored::Colorize;

use crate::addresses::add_address_name;
use crate::priority_transactions::{
    compute_merkle_tree, fetch_all_priority_transactions, PriorityTransaction,
};
use crate::sequencer::Sequencer;
use serde::Serialize;

fn format_address(value: Address) -> String {
    format!("{:#x}", value)
}

fn format_fixed_bytes(value: FixedBytes<32>) -> String {
    format!("{:#x}", value)
}

fn format_b256(value: B256) -> String {
    format!("{:#x}", value)
}

#[derive(Debug)]
pub struct StateTransition {
    verifier: Address,
    total_batches_executed: U256,
    total_batches_verified: U256,
    total_batches_committed: U256,
    bootloader_hash: FixedBytes<32>,
    default_account_hash: FixedBytes<32>,
    protocol_version: (u32, u32, u32),
    system_upgrade_tx_hash: FixedBytes<32>,
    admin: Address,
    chain_id: U256,
    settlement_layer: Address,

    unprocessed_queue_size: U256,
    total_queue_size: U256,
    priority_tree_root: B256,

    hyperchain: Address,
    base_token_asset_id: B256,
}

#[derive(Serialize)]
pub struct QueueReport {
    pub unprocessed: String,
    pub total: String,
}

#[derive(Serialize)]
pub struct StateTransitionReport {
    pub chain_id: String,
    pub hyperchain: String,
    pub verifier: String,
    pub total_batches_executed: String,
    pub total_batches_verified: String,
    pub total_batches_committed: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_batch_update_unix: Option<u64>,
    pub bootloader_hash: String,
    pub default_account_hash: String,
    pub protocol_version: (u32, u32, u32),
    pub system_upgrade_tx_hash: String,
    pub admin: String,
    pub settlement_layer: String,
    pub queue: QueueReport,
    pub priority_tree_root: String,
    pub base_token_asset_id: String,
}

sol! {
    #[sol(rpc)]
    contract IHyperchain {
        function getVerifier() external view returns (address);
        function getAdmin() external view returns (address);
        function getTotalBatchesCommitted() external view returns (uint256);
        function getTotalBatchesVerified() external view returns (uint256);
        function getTotalBatchesExecuted() external view returns (uint256);
        function getSemverProtocolVersion() external view returns (uint32, uint32, uint32);

        function getL2BootloaderBytecodeHash() external view returns (bytes32);
        function getL2DefaultAccountBytecodeHash() external view returns (bytes32);
        function getL2SystemContractsUpgradeTxHash() external view returns (bytes32);
        function getChainId() external view returns (uint256);
        function getSettlementLayer() external view returns (address);

        function getPriorityQueueSize() external view returns (uint256);
        function getTotalPriorityTxs() external view returns (uint256);
        function getPriorityTreeRoot() external view returns (bytes32);
        function getBaseTokenAssetId() external view returns (bytes32);


    }
}

fn mark_red_if_not_empty<T: std::fmt::Display + core::cmp::PartialEq>(
    address: T,
    empty: T,
) -> String {
    if address == empty {
        return address.to_string();
    }
    return format!("{}", address).red().to_string();
}

impl Display for StateTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.detailed_fmt(f, 0)
    }
}

impl StateTransition {
    pub async fn new(
        provider: &alloy::providers::RootProvider<
            alloy::transports::http::Http<alloy::transports::http::Client>,
        >,
        hyperchain: Address,
    ) -> eyre::Result<StateTransition> {
        let contract = IHyperchain::new(hyperchain, provider);

        let verifier = contract.getVerifier().call().await?._0;
        let total_batches_committed = contract.getTotalBatchesCommitted().call().await?._0;
        let total_batches_verified = contract.getTotalBatchesVerified().call().await?._0;
        let total_batches_executed = contract.getTotalBatchesExecuted().call().await?._0;
        let protocol_version = contract.getSemverProtocolVersion().call().await?;

        let admin = contract.getAdmin().call().await?._0;

        let bootloader_hash = contract.getL2BootloaderBytecodeHash().call().await?._0;
        let default_account_hash = contract.getL2DefaultAccountBytecodeHash().call().await?._0;
        let system_upgrade_tx_hash = contract
            .getL2SystemContractsUpgradeTxHash()
            .call()
            .await?
            ._0;

        let chain_id = contract.getChainId().call().await?._0;

        add_address_name(admin, format!("Admin {}", chain_id));
        let settlement_layer = contract.getSettlementLayer().call().await?._0;

        let unprocessed_queue_size = contract.getPriorityQueueSize().call().await?._0;
        let total_queue_size = contract.getTotalPriorityTxs().call().await?._0;

        let priority_tree_root = contract.getPriorityTreeRoot().call().await?._0;

        let base_token_asset_id = contract.getBaseTokenAssetId().call().await?._0;

        Ok(StateTransition {
            verifier,
            total_batches_executed,
            total_batches_verified,
            total_batches_committed,
            bootloader_hash,
            default_account_hash,
            protocol_version: (
                protocol_version._0,
                protocol_version._1,
                protocol_version._2,
            ),
            system_upgrade_tx_hash,
            admin,
            chain_id,
            settlement_layer,
            unprocessed_queue_size,
            total_queue_size,
            priority_tree_root,
            hyperchain,
            base_token_asset_id,
        })
    }

    pub fn total_batches_committed(&self) -> U256 {
        self.total_batches_committed
    }

    pub fn to_report(&self, last_batch_update_unix: Option<u64>) -> StateTransitionReport {
        StateTransitionReport {
            chain_id: self.chain_id.to_string(),
            hyperchain: format_address(self.hyperchain),
            verifier: format_address(self.verifier),
            total_batches_executed: self.total_batches_executed.to_string(),
            total_batches_verified: self.total_batches_verified.to_string(),
            total_batches_committed: self.total_batches_committed.to_string(),
            last_batch_update_unix,
            bootloader_hash: format_fixed_bytes(self.bootloader_hash),
            default_account_hash: format_fixed_bytes(self.default_account_hash),
            protocol_version: self.protocol_version,
            system_upgrade_tx_hash: format_fixed_bytes(self.system_upgrade_tx_hash),
            admin: format_address(self.admin),
            settlement_layer: format_address(self.settlement_layer),
            queue: QueueReport {
                unprocessed: self.unprocessed_queue_size.to_string(),
                total: self.total_queue_size.to_string(),
            },
            priority_tree_root: format_b256(self.priority_tree_root),
            base_token_asset_id: format_b256(self.base_token_asset_id),
        }
    }

    pub fn detailed_fmt(&self, f: &mut std::fmt::Formatter<'_>, pad: usize) -> std::fmt::Result {
        let pad = " ".repeat(pad);
        writeln!(f, "{}Chain id: {}", pad, self.chain_id)?;
        writeln!(
            f,
            "{}  Protocol version: {}.{}.{}",
            pad, self.protocol_version.0, self.protocol_version.1, self.protocol_version.2
        )?;
        writeln!(
            f,
            "{}  Batches (C,V,E):  {} {} {}",
            pad,
            self.total_batches_committed,
            self.total_batches_verified,
            self.total_batches_executed
        )?;

        writeln!(
            f,
            "{}  System upgrade:   {}",
            pad,
            mark_red_if_not_empty(self.system_upgrade_tx_hash, FixedBytes::<32>::ZERO)
        )?;
        writeln!(
            f,
            "{}  AA hash:          {}",
            pad,
            self.default_account_hash.to_string()
        )?;
        writeln!(f, "{}  Verifier:         {}", pad, self.verifier)?;
        writeln!(f, "{}  Admin:            {}", pad, self.admin)?;
        writeln!(
            f,
            "{}  Bootloader hash:  {}",
            pad,
            self.bootloader_hash.to_string()
        )?;

        writeln!(
            f,
            "{}  Settlement layer: {}",
            pad,
            mark_red_if_not_empty(self.settlement_layer, Address::ZERO)
        )?;

        writeln!(
            f,
            "{}  Queue unprocessed / total: {} / {}",
            pad, self.unprocessed_queue_size, self.total_queue_size
        )?;

        Ok(())
    }

    pub async fn get_priority_transactions(
        &self,
        sequencer: &Sequencer,
    ) -> eyre::Result<Vec<PriorityTransaction>> {
        fetch_all_priority_transactions(sequencer, self.hyperchain).await
    }

    pub async fn verify_priority_root_hash(&self, sequencer: &Sequencer) -> eyre::Result<()> {
        let txs = self.get_priority_transactions(sequencer).await?;
        if compute_merkle_tree(&txs) != self.priority_tree_root {
            eyre::bail!(
                "Priority tree root hash invalid: {} vs {}",
                self.priority_tree_root,
                compute_merkle_tree(&txs)
            )
        }

        Ok(())
    }
}
