use alloy::primitives::{address, Address, U256};
use alloy::sol;
use bridgehub::BridgehubSummary;
use clap::{Parser, ValueEnum};
use colored::Colorize;
use priority_transactions::PriorityTransactionReport;
use sequencer::{detect_sequencer, SequencerType};
use serde::Serialize;
use statetransition::{StateTransition, StateTransitionReport};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

mod addresses;
mod bridgehub;
mod l1_asset_router;
mod l2_asset_router;
mod priority_transactions;
mod sequencer;
mod statetransition;
mod stm;
mod utils;

use chrono::Utc;

sol! {
    #[sol(rpc)]
    contract SharedBridge {
        function assetHandlerAddress(bytes32 asset_id) public view returns (address) {}

    }
}

fn format_wei_amount(wei: &U256) -> String {
    let wei_string = wei.to_string();
    let len = wei_string.len();

    if len > 18 {
        // Insert a decimal 18 places from the end
        format!("{}.{}", &wei_string[..len - 18], &wei_string[len - 18..])
    } else {
        // If the string is shorter than 18 characters, pad with zeros
        format!(
            "0.{}",
            wei_string
                .chars()
                .rev()
                .chain("000000000000000000".chars())
                .take(18)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        )
    }
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    network: Option<Network>,

    #[arg(long)]
    bridgehub: Option<Address>,

    #[arg(long)]
    l1_url: Option<String>,

    #[arg(long, value_name = "PATH", default_value = "data/output.json")]
    output: PathBuf,

    #[arg(long, value_name = "PATH", default_value = "data/cache")]
    cache_dir: PathBuf,

    #[arg(long)]
    versioned_output: bool,

    /// Skip querying the L2 gateway (useful when gateway is deprecated/unavailable)
    #[arg(long)]
    no_gateway: bool,
}

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum Network {
    Local,
    Mainnet,
    Testnet,
    TestnetAtlas,
    Stage,
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Network::Local => "local",
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
            Network::TestnetAtlas => "testnet_atlas",
            Network::Stage => "stage",
        };
        write!(f, "{}", label)
    }
}

#[derive(Serialize)]
struct DiagnosticsReport {
    generated_at_unix: u64,
    network: String,
    sequencers: SequencersReport,
    bridgehub: BridgehubSummary,
    gateway_bridgehub: Option<BridgehubSummary>,
    l1_balances: Vec<ChainBalanceReport>,
    chains: Vec<ChainDiagnostics>,
}

#[derive(Serialize)]
struct SequencersReport {
    l1: SequencerStatus,
    l2: SequencerStatus,
    l3: SequencerStatus,
}

#[derive(Serialize)]
struct SequencerStatus {
    status: String,
    sequencer: Option<sequencer::Sequencer>,
    error: Option<String>,
}

impl SequencerStatus {
    fn ok(sequencer: sequencer::Sequencer) -> Self {
        Self {
            status: "ok".to_string(),
            sequencer: Some(sequencer),
            error: None,
        }
    }

    fn err(error: &eyre::Report) -> Self {
        Self {
            status: "error".to_string(),
            sequencer: None,
            error: Some(error.to_string()),
        }
    }
}

#[derive(Serialize)]
struct ChainBalanceReport {
    chain_id: u64,
    tokens: Vec<TokenBalanceReport>,
}

#[derive(Serialize)]
struct TokenBalanceReport {
    token: String,
    raw_wei: String,
    formatted: String,
}

#[derive(Serialize)]
struct ChainDiagnostics {
    chain_id: u64,
    state_transition: Option<StateTransitionReport>,
    state_transition_error: Option<String>,
    priority_tree_verified: Option<bool>,
    priority_tree_note: Option<String>,
    priority_transactions: Vec<PriorityTransactionReport>,
    priority_tx_error: Option<String>,
}

impl ChainDiagnostics {
    fn new(chain_id: u64) -> Self {
        Self {
            chain_id,
            state_transition: None,
            state_transition_error: None,
            priority_tree_verified: None,
            priority_tree_note: None,
            priority_transactions: Vec::new(),
            priority_tx_error: None,
        }
    }
}

fn resolve_output_path(base_path: &Path, versioned: bool) -> PathBuf {
    if !versioned {
        return base_path.to_path_buf();
    }

    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let stem = base_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let extension = base_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("json");
    let filename = format!("{stem}-{timestamp}.{extension}");

    match base_path.parent() {
        Some(parent) => parent.join(filename),
        None => PathBuf::from(filename),
    }
}

fn write_report(
    report: &DiagnosticsReport,
    base_path: &Path,
    versioned: bool,
) -> eyre::Result<PathBuf> {
    let target_path = resolve_output_path(base_path, versioned);

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let serialized = serde_json::to_vec_pretty(report)?;

    let tmp_extension = {
        let ext = target_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("json");
        if ext.is_empty() {
            "tmp".to_string()
        } else {
            format!("{ext}.tmp")
        }
    };
    let tmp_path = target_path.with_extension(tmp_extension);

    fs::write(&tmp_path, serialized)?;
    fs::rename(&tmp_path, &target_path)?;

    Ok(target_path)
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Cli::parse();

    let (l1_rpc, l2_rpc, l3_rpc) = match args.network.clone().unwrap_or(Network::Local) {
        Network::Local => (
            "http://127.0.0.1:8545",
            Some("http://127.0.0.1:3150"),
            "http://127.0.0.1:3050",
        ),
        Network::Mainnet => (
            //"https://rpc.flashbots.net",
            "https://eth.llamarpc.com",
            Some("https://rpc.era-gateway-mainnet.zksync.dev/"),
            "https://mainnet.era.zksync.io",
        ),
        Network::Stage => (
            "https://1rpc.io/sepolia",
            Some("https://rpc.era-gateway-stage.zksync.dev/"),
            "https://dev-api.era-stage-proofs.zksync.dev/",
        ),
        Network::Testnet => (
            "https://1rpc.io/sepolia",
            // TODO: for testnet, we'll have to point at the new testnet gateway once it's live
            Some("https://rpc.era-gateway-testnet.zksync.dev/"),
            "https://sepolia.era.zksync.dev",
        ),
        Network::TestnetAtlas => (
            "https://1rpc.io/sepolia",
            // Update once gateway launches there.
            Some("https://zksync-os-testnet-alpha.zksync.dev/"),
            "https://zksync-os-testnet-alpha.zksync.dev/",
        ),
    };

    let l2_rpc = if args.no_gateway { None } else { l2_rpc };
    let l1_rpc = args.l1_url.as_deref().unwrap_or(l1_rpc);

    println!("====================================");
    println!("=====   Elastic chain debugger =====");
    println!("====================================");

    let l1_sequencer = detect_sequencer(l1_rpc).await?;

    println!("{} L1 (ethereum) - {}", "[OK]".green(), l1_sequencer);

    let l2_sequencer = match l2_rpc {
        Some(url) => {
            let result = detect_sequencer(url).await;
            match &result {
                Ok(l2_sequencer) => {
                    println!("{} L2 (sequencer) - {}", "[OK]".green(), l2_sequencer)
                }
                Err(err) => println!("{} L2 (sequencer) - {}", "[ERROR]".red(), err),
            };
            Some(result)
        }
        None => {
            println!(
                "{} L2 (gateway) - skipped (--no-gateway)",
                "[SKIP]".yellow()
            );
            None
        }
    };

    // The client sequencer might not be running - but that's ok.
    let l3_sequencer = detect_sequencer(l3_rpc).await;
    match &l3_sequencer {
        Ok(l3_sequencer) => println!("{} L3 (client)   - {}", "[OK]".green(), l3_sequencer),
        Err(err) => println!("{} L3 (client)   - {}", "[ERROR]".red(), err),
    };

    let bridgehub_address = match &l2_sequencer {
        Some(Ok(l2_sequencer)) => {
            if let SequencerType::L2(info) = &l2_sequencer.sequencer_type {
                info.bridgehub_address
            } else {
                eyre::bail!("port 3050 doesn't have zksync sequencer");
            }
        }
        Some(Err(_)) | None => {
            println!(
                "{} L2 (sequencer) missing - using L3 sequencer instead",
                "[ERROR]".red(),
            );
            if let Ok(l3_sequencer) = &l3_sequencer {
                if let SequencerType::L2(info) = &l3_sequencer.sequencer_type {
                    info.bridgehub_address
                } else {
                    eyre::bail!("port 3050 doesn't have zksync sequencer");
                }
            } else {
                eyre::bail!(
                    "L2 sequencer is not available and L3 sequencer is not a valid L2 sequencer"
                );
            }
        }
    };

    let bridgehub = bridgehub::Bridgehub::new(
        &l1_sequencer,
        args.bridgehub.unwrap_or(bridgehub_address),
        &args.cache_dir,
    )
    .await?;

    println!("===");
    println!("=== {} ", format!("Bridgehub - L1").bold().green());
    println!("===");

    println!("{}", bridgehub);

    println!("=== Bridgehub chains");
    bridgehub.print_detailed_info().await?;

    println!("=== Balances ");

    let balances = bridgehub.get_all_chains_balances(&l1_sequencer).await?;

    let mut balance_reports = Vec::new();
    let mut sorted_balance_keys: Vec<u64> = balances.keys().copied().collect();
    sorted_balance_keys.sort_unstable();
    for chain in sorted_balance_keys {
        if let Some(balance) = balances.get(&chain) {
            println!("   Chain : {}", format!("{}", chain).bold());

            let mut token_reports = Vec::new();
            let mut tokens: Vec<_> = balance.iter().collect();
            tokens.sort_by(|a, b| a.0.cmp(b.0));
            for (token, amount) in tokens {
                println!(
                    "      {:<20} : {:>28}",
                    token.bold(),
                    format_wei_amount(amount)
                );
                token_reports.push(TokenBalanceReport {
                    token: token.clone(),
                    raw_wei: amount.to_string(),
                    formatted: format_wei_amount(amount),
                });
            }
            balance_reports.push(ChainBalanceReport {
                chain_id: chain,
                tokens: token_reports,
            });
        }
    }

    let gateway_bridgehub = match &l2_sequencer {
        Some(Ok(l2_sequencer)) => {
            let gateway_bridgehub_address = address!("0000000000000000000000000000000000010002");
            let gateway_bridgehub =
                bridgehub::Bridgehub::new(l2_sequencer, gateway_bridgehub_address, &args.cache_dir)
                    .await?;

            println!("===");
            println!("=== {} ", format!("Bridgehub - Gateway").bold().green());
            println!("===");

            println!("{}", gateway_bridgehub);

            println!("\n=== Chains");
            gateway_bridgehub.print_detailed_info().await?;

            println!("===");
            println!("=== {} ", format!("ST / Hyperchains").bold().green());
            println!("===");
            Some(gateway_bridgehub)
        }
        _ => None,
    };

    let bridgehub_summary = bridgehub.to_summary();
    let gateway_summary = gateway_bridgehub.as_ref().map(|g| g.to_summary());

    let mut chain_reports: BTreeMap<u64, ChainDiagnostics> = BTreeMap::new();
    let mut state_transitions: BTreeMap<u64, StateTransition> = BTreeMap::new();
    let mut sorted_chains: Vec<u64> = bridgehub.known_chains.iter().copied().collect();
    sorted_chains.sort_unstable();

    for (i, chain) in sorted_chains.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let mut diagnostics = ChainDiagnostics::new(*chain);
        let st = bridgehub.get_state_transition(*chain).await;

        match st {
            Ok(st) => {
                print!("Chain {} on L1: {}", chain, &st);
                diagnostics.state_transition = Some(st.to_report());
                if args.network.as_ref().unwrap_or(&Network::Local) == &Network::Local {
                    st.verify_priority_root_hash(&l1_sequencer, &args.cache_dir).await?;
                    println!("  Priority tree hash: {}", "VALID".green());
                    diagnostics.priority_tree_verified = Some(true);
                } else {
                    println!("  Skipping priority hash verification on non-local chains.");
                    diagnostics.priority_tree_note = Some(
                        "Skipped priority hash verification on non-local networks.".to_string(),
                    );
                }
                state_transitions.insert(*chain, st);
            }
            Err(err) => {
                println!("Failed to get info for Chain {} on L1: {}", chain, err);
                diagnostics.state_transition_error = Some(err.to_string());
            }
        }

        println!("");
        chain_reports.insert(*chain, diagnostics);
    }

    if let Some(gateway_bridgehub) = &gateway_bridgehub {
        for chain in &gateway_bridgehub.known_chains {
            println!(
                "Chain {} on Gateway: {}",
                chain,
                gateway_bridgehub.get_state_transition(*chain).await?
            );
        }
    }

    println!("===");
    println!("=== {} ", format!("Priority TXs").bold().green());
    println!("===");

    for (i, chain) in sorted_chains.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        println!("Chain {}", chain);

        if let Some(st) = state_transitions.get(chain) {
            match st.get_priority_transactions(&l1_sequencer, &args.cache_dir).await {
                Ok(mut txs) => {
                    txs.sort_by_key(|x| x.index);
                    for tx in &txs {
                        println!("{}", tx);
                    }
                    println!("");
                    if let Some(report) = chain_reports.get_mut(chain) {
                        report.priority_transactions =
                            txs.into_iter().map(|tx| tx.to_report()).collect();
                    }
                }
                Err(err) => {
                    println!("  Error fetching priority txs: {}", err);
                    if let Some(report) = chain_reports.get_mut(chain) {
                        report.priority_tx_error = Some(err.to_string());
                    }
                }
            }
        } else if let Some(report) = chain_reports.get_mut(chain) {
            let message = "State transition details not available".to_string();
            report.priority_tx_error = Some(message.clone());
            println!("  {}", message);
        }
    }

    let generated_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let sequencers_report = SequencersReport {
        l1: SequencerStatus::ok(l1_sequencer.clone()),
        l2: match &l2_sequencer {
            Some(Ok(seq)) => SequencerStatus::ok(seq.clone()),
            Some(Err(err)) => SequencerStatus::err(err),
            None => SequencerStatus {
                status: "skipped".to_string(),
                sequencer: None,
                error: None,
            },
        },
        l3: match &l3_sequencer {
            Ok(seq) => SequencerStatus::ok(seq.clone()),
            Err(err) => SequencerStatus::err(err),
        },
    };

    let diagnostics = DiagnosticsReport {
        generated_at_unix,
        network: args.network.clone().unwrap_or(Network::Local).to_string(),
        sequencers: sequencers_report,
        bridgehub: bridgehub_summary,
        gateway_bridgehub: gateway_summary,
        l1_balances: balance_reports,
        chains: chain_reports.into_values().collect(),
    };

    let output_path = write_report(&diagnostics, &args.output, args.versioned_output)?;
    println!(
        "Serialized diagnostics report saved to {}",
        output_path.display()
    );

    Ok(())
}
