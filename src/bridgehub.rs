use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::path::Path;

use crate::l1_asset_router::{AssetHandler, L1AssetRouter};
use crate::l2_asset_router::L2AssetRouter;
use crate::sequencer::Sequencer;
use crate::statetransition::StateTransition;
use crate::stm::ChainTypeManager;
use crate::utils::get_human_name_for;
use alloy::primitives::{Address, FixedBytes, U256};
use alloy::providers::{Provider, RootProvider};
use alloy::sol;
use alloy::transports::http::{Client, Http};
use colored::Colorize;

use futures::future::join_all;
use serde::Serialize;

fn format_address(value: Address) -> String {
    format!("{:#x}", value)
}

fn format_fixed_bytes(value: FixedBytes<32>) -> String {
    format!("{:#x}", value)
}

#[derive(Serialize)]
pub struct BridgehubSummary {
    pub address: String,
    pub shared_bridge: String,
    pub ctm_deployer: String,
    pub known_chains: Vec<u64>,
    pub ctms: Option<Vec<ChainTypeManagerSummary>>,
    pub asset_router: AssetRouterSummary,
}

#[derive(Serialize)]
pub struct ChainTypeManagerSummary {
    pub address: String,
    pub bridgehub: String,
    pub admin: String,
    pub owner: String,
    pub asset_id: String,
    pub asset_name: String,
}

#[derive(Serialize)]
pub struct RegisteredAssetSummary {
    pub asset_id: String,
    pub name: String,
    pub handler: String,
    pub token_address: Option<String>,
    pub token_name: Option<String>,
    pub handler_address: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssetRouterSummary {
    L1 {
        address: String,
        native_token_vault: String,
        registered_assets: Vec<RegisteredAssetSummary>,
    },
    L2 {
        address: String,
    },
}

impl From<&ChainTypeManager> for ChainTypeManagerSummary {
    fn from(value: &ChainTypeManager) -> Self {
        ChainTypeManagerSummary {
            address: format_address(value.address),
            bridgehub: format_address(value.bridgehub),
            admin: format_address(value.admin),
            owner: format_address(value.owner),
            asset_id: format_fixed_bytes(value.asset_id),
            asset_name: value.asset_name.clone(),
        }
    }
}

impl From<&crate::l1_asset_router::RegisteredAsset> for RegisteredAssetSummary {
    fn from(value: &crate::l1_asset_router::RegisteredAsset) -> Self {
        let name = value.name();
        match &value.handler {
            AssetHandler::Bridgehub => RegisteredAssetSummary {
                asset_id: format_fixed_bytes(value.asset_id),
                name,
                handler: "bridgehub".to_string(),
                token_address: None,
                token_name: None,
                handler_address: None,
            },
            AssetHandler::NativeTokenVault(asset) => RegisteredAssetSummary {
                asset_id: format_fixed_bytes(value.asset_id),
                name,
                handler: "native_token_vault".to_string(),
                token_address: Some(format_address(asset.address)),
                token_name: Some(asset.token_name.clone()),
                handler_address: None,
            },
            AssetHandler::Other(address) => RegisteredAssetSummary {
                asset_id: format_fixed_bytes(value.asset_id),
                name,
                handler: "other".to_string(),
                token_address: None,
                token_name: None,
                handler_address: Some(format_address(*address)),
            },
        }
    }
}

sol! {
    #[sol(rpc)]
    contract IBridgehub {
        address public sharedBridge;
        mapping(uint256 chainId => address) public chainTypeManager;
        mapping(uint256 chainId => address) public baseToken;
        function getHyperchain(uint256 _chainId) public view returns (address) {}
        function ctmAssetIdFromChainId(uint256 chain_id) public view returns (bytes32) {}

        function ctmAssetIdFromAddress(address) public view returns (bytes32) {}

        event NewChain(uint256 indexed chainId, address chainTypeManager, address indexed chainGovernance);
        event AssetRegistered(
            bytes32 indexed assetInfo,
            address indexed _assetAddress,
            bytes32 indexed additionalData,
            address sender
        );

        event ChainTypeManagerAdded(address indexed chainTypeManager);

        event ChainTypeManagerRemoved(address indexed chainTypeManager);

        function getAllZKChainChainIDs() external view returns (uint256[] memory);

        address public l1CtmDeployer;
    }
}

sol! {
    #[sol(rpc)]
    contract IValidatorTimelock {
        function getCommittedBatchTimestamp(uint256 chainId, uint256 batchNumber)
            external
            view
            returns (uint256);
    }
}

// Information about a single chain that is connected to a bridgehub.
// The chain_id is supposed to be a globally unique identifier.
// Note, that this object might exist in 'passive' mode - if the chain has migrated to a different sync layer.
pub struct BridgehubChainDetails {
    pub stm_address: Address,
    pub st_address: Address,
    pub base_token_address: Address,
    pub validator_timelock_address: Address,
    pub validator_timelock_post_v29_address: Address,
    pub stm_asset_id: FixedBytes<32>,
}

impl Display for BridgehubChainDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "    CTM:      {}",
            get_human_name_for(self.stm_asset_id).bold()
        )?;
        writeln!(f, "    CTM:                {}", self.stm_address)?;
        writeln!(f, "    ST:                 {}", self.st_address)?;
        writeln!(f, "    Base Token:         {}", self.base_token_address)?;
        writeln!(
            f,
            "    Validator timelock: {}",
            self.validator_timelock_address
        )?;
        writeln!(
            f,
            "    Validator timelock post-v29: {}",
            self.validator_timelock_post_v29_address
        )?;
        Ok(())
    }
}

pub struct ValidatorTimelockPostingAccounts {
    pub committers: Vec<Address>,
    pub provers: Vec<Address>,
}

pub enum AssetRouter {
    L1(L1AssetRouter),
    L2(L2AssetRouter),
}

impl Display for AssetRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.detailed_fmt(f, 0)
    }
}

impl AssetRouter {
    fn detailed_fmt(&self, f: &mut std::fmt::Formatter<'_>, pad_size: usize) -> std::fmt::Result {
        let pad = " ".repeat(pad_size);
        match self {
            AssetRouter::L1(router) => {
                writeln!(f, "{}L1 asset router", pad)?;
                router.detailed_fmt(f, pad_size + 3)?;
            }
            AssetRouter::L2(router) => {
                writeln!(f, "{}L2 asset router", pad)?;
                router.detailed_fmt(f, pad_size + 3)?;
            }
        }

        Ok(())
    }
}

/// Bridgehub is the main coordination contract on each chain.
/// the 'main main' bridgehub is located on L1.
pub struct Bridgehub {
    pub address: Address,
    pub shared_bridge: Address,
    pub known_chains: HashSet<u64>,
    pub ctms: Option<Vec<ChainTypeManager>>,
    provider: RootProvider<Http<Client>>,
    pub ctm_deployer: Address,

    pub asset_router: AssetRouter,
}

impl Display for Bridgehub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "   Bridgehub at          {}", self.address,)?;
        writeln!(f, "   Shared bridge:        {}", self.shared_bridge)?;
        writeln!(f, "   CTM deployer (on L1): {}", self.ctm_deployer)?;
        if let Some(ctms) = &self.ctms {
            writeln!(f, "   CTMS: {}", ctms.len())?;

            for stm in ctms {
                stm.detailed_fmt(f, 3)?;
            }
        }

        writeln!(f, "   === Asset router")?;
        self.asset_router.detailed_fmt(f, 3)?;

        Ok(())
    }
}

impl Bridgehub {
    pub fn to_summary(&self) -> BridgehubSummary {
        let mut known_chains: Vec<u64> = self.known_chains.iter().copied().collect();
        known_chains.sort_unstable();

        let ctms = self.ctms.as_ref().map(|ctms| {
            let mut summaries: Vec<_> = ctms.iter().map(ChainTypeManagerSummary::from).collect();
            summaries.sort_by(|a, b| a.asset_name.cmp(&b.asset_name));
            summaries
        });

        let asset_router = match &self.asset_router {
            AssetRouter::L1(router) => {
                let mut assets: Vec<_> = router.registered_assets.values().collect();
                assets.sort_by(|a, b| a.name().cmp(&b.name()));
                let registered_assets = assets
                    .into_iter()
                    .map(RegisteredAssetSummary::from)
                    .collect();

                AssetRouterSummary::L1 {
                    address: format_address(router.address),
                    native_token_vault: format_address(router.native_token_vault),
                    registered_assets,
                }
            }
            AssetRouter::L2(router) => AssetRouterSummary::L2 {
                address: format_address(router.address),
            },
        };

        BridgehubSummary {
            address: format_address(self.address),
            shared_bridge: format_address(self.shared_bridge),
            ctm_deployer: format_address(self.ctm_deployer),
            known_chains,
            ctms,
            asset_router,
        }
    }

    pub async fn new(
        sequencer: &Sequencer,
        address: Address,
        cache_dir: &Path,
    ) -> eyre::Result<Bridgehub> {
        let provider = sequencer.get_provider();

        let data = provider.get_code_at(address).await?;
        if data.len() == 0 {
            // empty contract - something's wrong.
            eyre::bail!(
                "Trying to read bridgehub data from address {} at {}, but code is empty. Is it a rigth address on right chain?",
                address,
                provider.client().transport().url()
            );
        }

        let contract = IBridgehub::new(address, provider);
        let shared_bridge = contract.sharedBridge().call().await?.sharedBridge;

        let known_chains = contract.getAllZKChainChainIDs().call().await?._0;

        let known_chains: HashSet<u64> =
            known_chains.iter().map(|x| x.try_into().unwrap()).collect();

        let ctm_deployer = contract.l1CtmDeployer().call().await?.l1CtmDeployer;

        let mut ctm_addresses = HashSet::new();

        for chain_id in known_chains.iter() {
            let aa = contract
                .chainTypeManager(U256::from(*chain_id))
                .call()
                .await
                .map(|x| x._0)?;
            ctm_addresses.insert(aa);
        }

        let ctms = {
            let stms = ctm_addresses
                .into_iter()
                .map(|address| ChainTypeManager::new(sequencer, address));
            let stms: Vec<_> = join_all(stms)
                .await
                .into_iter()
                .filter_map(|r| r.ok())
                .collect();
            Some(stms)
        };

        let asset_router = match sequencer.sequencer_type {
            crate::sequencer::SequencerType::L1 => {
                AssetRouter::L1(L1AssetRouter::new(sequencer, shared_bridge, cache_dir).await?)
            }
            crate::sequencer::SequencerType::L2(_) => {
                AssetRouter::L2(L2AssetRouter::new(sequencer, shared_bridge).await)
            }
        };

        Ok(Bridgehub {
            address,
            shared_bridge,
            known_chains,
            provider: sequencer.get_provider(),
            ctms,
            ctm_deployer,
            asset_router,
        })
    }

    pub async fn print_detailed_info(&self) -> eyre::Result<()> {
        println!("  Bridgehub:          {}", self.address);

        for chain_id in &self.known_chains {
            println!("{}", format!("  Chain: {:?}", chain_id).bold());
            let details = self.get_chain_details(*chain_id).await?;
            println!("{}", details);
        }

        Ok(())
    }

    pub async fn get_chain_details(&self, chain_id: u64) -> eyre::Result<BridgehubChainDetails> {
        sol! {
            #[sol(rpc)]
            contract IChainTypeManager {
                address public validatorTimelock;
                address public validatorTimelockPostV29;
            }
        }

        let contract = IBridgehub::new(self.address, &self.provider);

        let stm_address = contract
            .chainTypeManager(U256::from(chain_id))
            .call()
            .await?
            ._0;

        let base_token_address = match contract.baseToken(U256::from(chain_id)).call().await {
            Ok(base_token) => base_token._0,
            // FIXME: remove after we fix an issue where basetoken is not set after migration.
            Err(_) => Address::ZERO,
        };
        let st_address = contract
            .getHyperchain(U256::from(chain_id))
            .call()
            .await?
            ._0;

        let stm_contract = IChainTypeManager::new(stm_address, &self.provider);

        let validator_timelock_address = stm_contract
            .validatorTimelock()
            .call()
            .await?
            .validatorTimelock;

        let validator_timelock_post_v29_address = stm_contract
            .validatorTimelockPostV29()
            .call()
            .await?
            .validatorTimelockPostV29;

        let asset_id = contract
            .ctmAssetIdFromChainId(U256::from(chain_id))
            .call()
            .await?
            ._0;

        Ok(BridgehubChainDetails {
            stm_address,
            st_address,
            base_token_address,
            validator_timelock_address,
            validator_timelock_post_v29_address,
            stm_asset_id: asset_id,
        })
    }

    pub async fn get_validator_timelock_posting_accounts(
        &self,
        validator_timelock_address: Address,
        chain_address: Address,
    ) -> eyre::Result<ValidatorTimelockPostingAccounts> {
        sol! {
            #[sol(rpc)]
            contract IValidatorTimelock {
                function COMMITTER_ROLE() external view returns (bytes32);
                function PROVER_ROLE() external view returns (bytes32);
                function getRoleMemberCount(address _chainAddress, bytes32 _role) external view returns (uint256);
                function getRoleMember(address _chainAddress, bytes32 _role, uint256 _index) external view returns (address);
            }
        }

        if validator_timelock_address == Address::ZERO {
            return Ok(ValidatorTimelockPostingAccounts {
                committers: Vec::new(),
                provers: Vec::new(),
            });
        }

        let contract = IValidatorTimelock::new(validator_timelock_address, &self.provider);
        let committer_role = contract.COMMITTER_ROLE().call().await?._0;
        let prover_role = contract.PROVER_ROLE().call().await?._0;

        let committer_count = contract
            .getRoleMemberCount(chain_address, committer_role)
            .call()
            .await?
            ._0
            .try_into()?;
        let prover_count = contract
            .getRoleMemberCount(chain_address, prover_role)
            .call()
            .await?
            ._0
            .try_into()?;

        let mut committers = Vec::new();
        for i in 0..committer_count {
            committers.push(
                contract
                    .getRoleMember(chain_address, committer_role, U256::from(i))
                    .call()
                    .await?
                    ._0,
            );
        }

        let mut provers = Vec::new();
        for i in 0..prover_count {
            provers.push(
                contract
                    .getRoleMember(chain_address, prover_role, U256::from(i))
                    .call()
                    .await?
                    ._0,
            );
        }

        Ok(ValidatorTimelockPostingAccounts {
            committers,
            provers,
        })
    }

    pub async fn get_state_transition(&self, chain_id: u64) -> eyre::Result<StateTransition> {
        let contract = IBridgehub::new(self.address, &self.provider);

        let st_address = contract
            .getHyperchain(U256::from(chain_id))
            .call()
            .await?
            ._0;
        StateTransition::new(&self.provider, st_address).await
    }

    pub async fn get_committed_batch_timestamp(
        &self,
        chain_id: u64,
        batch_number: U256,
    ) -> eyre::Result<Option<u64>> {
        if batch_number.is_zero() {
            return Ok(None);
        }

        let details = self.get_chain_details(chain_id).await?;
        if details.validator_timelock_address == Address::ZERO {
            return Ok(None);
        }

        let timelock = IValidatorTimelock::new(details.validator_timelock_address, &self.provider);
        let timestamp = timelock
            .getCommittedBatchTimestamp(U256::from(chain_id), batch_number)
            .call()
            .await?
            ._0;

        if timestamp.is_zero() {
            return Ok(None);
        }

        Ok(Some(u64::try_from(timestamp).map_err(|_| {
            eyre::eyre!("Committed batch timestamp does not fit into u64")
        })?))
    }

    pub async fn get_all_chains_balances(
        &self,
        sequencer: &Sequencer,
    ) -> eyre::Result<HashMap<u64, HashMap<String, U256>>> {
        let mut result = HashMap::new();

        for chain_id in &self.known_chains {
            let foo = self.get_chain_balances(sequencer, *chain_id).await?;
            result.insert(*chain_id, foo);
        }

        Ok(result)
    }

    pub async fn get_chain_balances(
        &self,
        sequencer: &Sequencer,
        chain_id: u64,
    ) -> eyre::Result<HashMap<String, U256>> {
        let mut result = HashMap::new();
        match &self.asset_router {
            AssetRouter::L1(router) => {
                let assets = router.registered_assets.iter().filter_map(|(k, x)| {
                    if let AssetHandler::NativeTokenVault(_) = &x.handler {
                        Some((k, x))
                    } else {
                        None
                    }
                });
                for (asset_id, asset) in assets {
                    let amount = router
                        .chain_balance(sequencer, chain_id.try_into().unwrap(), asset_id)
                        .await;

                    result.insert(asset.name(), amount);
                }
            }

            AssetRouter::L2(_) => eyre::bail!("Not implemented yet"),
        };

        Ok(result)
    }
}
