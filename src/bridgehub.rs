use std::collections::HashSet;
use std::fmt::Display;

use crate::sequencer::Sequencer;
use crate::statetransition::StateTransition;
use crate::stm::detect_hyperchains;
use alloy::primitives::{Address, FixedBytes, U256};
use alloy::providers::{Provider, RootProvider};
use alloy::rpc::types::Filter;
use alloy::sol;
use alloy::sol_types::SolEvent;
use alloy::transports::http::{Client, Http};
use colored::Colorize;
use eyre::OptionExt;

sol! {
    #[sol(rpc)]
    contract IBridgehub {
        address public sharedBridge;
        mapping(uint256 chainId => address) public stateTransitionManager;
        mapping(uint256 chainId => address) public baseToken;
        function getHyperchain(uint256 _chainId) public view returns (address) {}
        function stmAssetIdFromChainId(uint256 chain_id) public view returns (bytes32) {}

        event NewChain(uint256 indexed chainId, address stateTransitionManager, address indexed chainGovernance);
        event AssetRegistered(
            bytes32 indexed assetInfo,
            address indexed _assetAddress,
            bytes32 indexed additionalData,
            address sender
        );
    }
}

pub struct BridgehubChainDetails {
    pub stm_address: Address,
    pub st_address: Address,
    pub shared_bridge_address: Address,
    pub base_token_address: Address,
    pub validator_timelock_address: Address,
    pub stm_asset_id: FixedBytes<32>,
}

impl Display for BridgehubChainDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "    Shared bridge:      {}", self.shared_bridge_address)?;
        writeln!(f, "    STM:                {}", self.stm_address)?;
        writeln!(f, "    ST:                 {}", self.st_address)?;
        writeln!(f, "    Base Token:         {}", self.base_token_address)?;
        writeln!(
            f,
            "    Validator timelock: {}",
            self.validator_timelock_address
        )?;
        writeln!(f, "    STM Asset id:       {}", self.stm_asset_id)?;
        Ok(())
    }
}

pub struct Bridgehub {
    pub address: Address,
    pub shared_bridge: Address,
    pub known_chains: Option<HashSet<u64>>,
    provider: RootProvider<Http<Client>>,
}

impl Display for Bridgehub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Bridgehub at {}. Shared bridge: {}",
            self.address, self.shared_bridge
        )
    }
}

impl Bridgehub {
    pub async fn new(
        sequencer: &Sequencer,
        address: Address,
        autodetect_chains: bool,
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

        let known_chains = if autodetect_chains {
            Some(Bridgehub::detect_chains(sequencer, address).await?)
        } else {
            None
        };

        Ok(Bridgehub {
            address,
            shared_bridge,
            known_chains,
            provider: sequencer.get_provider(),
        })
    }
    pub async fn detect_chains(
        sequencer: &Sequencer,
        bridgehub: Address,
    ) -> eyre::Result<HashSet<u64>> {
        match sequencer.sequencer_type {
            crate::sequencer::SequencerType::L1 => {
                Bridgehub::detect_chains_with_newchain_event(sequencer, bridgehub).await
            }
            crate::sequencer::SequencerType::L2(_) => {
                // For L2, we have to depend on NewHyperchain.
                let hyperchains = detect_hyperchains(sequencer).await?;
                Ok(hyperchains.iter().map(|(chain, _)| chain.clone()).collect())
            }
        }
    }

    async fn detect_chains_with_newchain_event(
        sequencer: &Sequencer,
        bridgehub: Address,
    ) -> eyre::Result<HashSet<u64>> {
        let provider = sequencer.get_provider();
        let mut current_block = provider.get_block_number().await?;
        let mut known_chains = HashSet::new();

        while current_block > 0 {
            let prev_limit = current_block.saturating_sub(10000);

            let filter = Filter::new()
                .from_block(prev_limit + 1)
                .to_block(current_block)
                .address(bridgehub);

            let logs = sequencer.get_provider().get_logs(&filter).await?;
            for log in logs {
                match log.topic0() {
                    Some(&IBridgehub::NewChain::SIGNATURE_HASH) => {
                        let chain_id: U256 = log.topics()[1].into();
                        known_chains.insert(chain_id.to::<u64>());
                    }

                    Some(&IBridgehub::AssetRegistered::SIGNATURE_HASH) => {
                        // TODO: do something with assets.
                        //println!("New asset. {:?}", log);
                    }
                    _ => (),
                }
            }
            current_block = prev_limit;
        }

        Ok(known_chains)
    }

    pub async fn print_detailed_info(&self) -> eyre::Result<()> {
        let chains = self.known_chains.clone().ok_or_eyre("chains not scanned")?;

        println!("  Bridgehub:          {}", self.address);

        for chain_id in chains {
            println!("{}", format!("  Chain: {:?}", chain_id).bold());
            let details = self.get_chain_details(chain_id).await?;
            println!("{}", details);
        }

        Ok(())
    }

    pub async fn get_chain_details(&self, chain_id: u64) -> eyre::Result<BridgehubChainDetails> {
        sol! {
            #[sol(rpc)]
            contract StateTransitionManager {
                address public validatorTimelock;
            }
        }

        let contract = IBridgehub::new(self.address, &self.provider);

        let stm_address = contract
            .stateTransitionManager(U256::from(chain_id))
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
        let shared_bridge_address = contract.sharedBridge().call().await?.sharedBridge;

        let stm_contract = StateTransitionManager::new(stm_address, &self.provider);

        let validator_timelock_address = stm_contract
            .validatorTimelock()
            .call()
            .await?
            .validatorTimelock;

        let asset_id = contract
            .stmAssetIdFromChainId(U256::from(chain_id))
            .call()
            .await?
            ._0;

        Ok(BridgehubChainDetails {
            stm_address,
            st_address,
            shared_bridge_address,
            base_token_address,
            validator_timelock_address,
            stm_asset_id: asset_id,
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
}
