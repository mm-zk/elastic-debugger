import { useEffect, useMemo, useState } from 'react';
import chainIdsDoc from './chain_ids.json';
import './App.css';

// Helpers
const shorten = (v) => {
  if (!v || typeof v !== 'string') return String(v ?? '');
  if (!v.startsWith('0x') || v.length <= 12) return v;
  return `${v.slice(0, 8)}…${v.slice(-6)}`;
};

const formatVersion = (ver) => {
  if (!ver) return 'n/a';
  const [a, b, c] = ver;
  return `${a}.${b}.${c}`;
};

const formatRelativeToRefresh = (timestampUnix, refreshUnix) => {
  if (!Number.isFinite(timestampUnix) || !Number.isFinite(refreshUnix)) return null;

  const diffSeconds = Math.round(refreshUnix - timestampUnix);
  if (Math.abs(diffSeconds) < 30) return 'at refresh';

  const thresholds = [
    ['day', 86_400],
    ['hour', 3_600],
    ['minute', 60],
    ['second', 1]
  ];

  for (const [unit, seconds] of thresholds) {
    if (Math.abs(diffSeconds) >= seconds) {
      const value = Math.round(Math.abs(diffSeconds) / seconds);
      const label = `${value} ${unit}${value === 1 ? '' : 's'}`;
      return `${label} ${diffSeconds > 0 ? 'before' : 'after'} refresh`;
    }
  }
  return null;
};

const formatTimestampTitle = (timestampUnix) => {
  if (!Number.isFinite(timestampUnix)) return null;
  return new Intl.DateTimeFormat('en', {
    dateStyle: 'medium',
    timeStyle: 'medium',
    timeZoneName: 'short'
  }).format(new Date(timestampUnix * 1000));
};

// Etherscan base URLs per ecosystem
const ETHERSCAN_BASES = {
  mainnet: 'https://etherscan.io/address/',
  testnet: 'https://sepolia.etherscan.io/address/',
  testnet_atlas: 'https://sepolia.etherscan.io/address/'
};

function Badge({ tone = 'neutral', children }) {
  return <span className={`badge badge--${tone}`}>{children}</span>;
}

function Collapsible({ title, defaultOpen = false, children, count }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="collapsible">
      <button className="collapsible__trigger" onClick={() => setOpen((v) => !v)}>
        <span className={`chevron ${open ? 'open' : ''}`}>▸</span>
        <span>{title}</span>
        {typeof count === 'number' && <span className="muted">({count})</span>}
      </button>
      {open && <div className="collapsible__content">{children}</div>}
    </div>
  );
}

function NodeBox({ title, subtitle, status, details, arrowTo }) {
  return (
    <section className="node card">
      <header className="node__header">
        <h2 className="node__title">{title}</h2>
        {subtitle && <div className="node__subtitle">{subtitle}</div>}
        {status && <Badge tone={status === 'ok' ? 'success' : 'danger'}>{status.toUpperCase()}</Badge>}
      </header>
      {arrowTo && (
        <div className="arrow">
          <span className="arrow__label">settles to</span>
          <span className="arrow__icon">→</span>
          <span className="arrow__target">{arrowTo}</span>
        </div>
      )}
      {details}
    </section>
  );
}

// Clickable Etherscan link for addresses (top-level)
function AddressLink({ address, eco }) {
  if (!address || typeof address !== 'string' || !address.startsWith('0x')) return <span>{address}</span>;
  const base = ETHERSCAN_BASES[eco] || ETHERSCAN_BASES.mainnet;
  return (
    <a
      href={`${base}${address}`}
      target="_blank"
      rel="noopener noreferrer"
      className="address-link"
      title={address}
    >
      {shorten(address)}
    </a>
  );
}

// Key/value row with address auto-linking (top-level)
function KeyValue({ label, value, eco }) {
  const isAddr = typeof value === 'string' && value.startsWith('0x') && value.length === 42;
  return (
    <div className="kv">
      <div className="kv__k">{label}</div>
      <div className="kv__v">{isAddr ? <AddressLink address={value} eco={eco} /> : value ?? <span className="muted">N/A</span>}</div>
    </div>
  );
}

function AddressList({ label, addresses, eco }) {
  const rows = (addresses || []).filter(Boolean);
  return (
    <div className="kv">
      <div className="kv__k">{label}</div>
      {rows.length > 0 ? (
        <div className="address-list">
          {rows.map((address) => (
            <a
              key={`${label}-${address}`}
              href={`${ETHERSCAN_BASES[eco] || ETHERSCAN_BASES.mainnet}${address}`}
              target="_blank"
              rel="noopener noreferrer"
              className="address-chip"
              title={address}
            >
              {shorten(address)}
            </a>
          ))}
        </div>
      ) : (
        <div className="muted">None</div>
      )}
    </div>
  );
}

function PriorityTable({ txs, eco }) {
  if (!txs || txs.length === 0) {
    return <div className="muted">No priority transactions</div>;
  }
  const extractAddr = (v) => {
    if (typeof v !== 'string') return v;
    const m = v.match(/(0x[a-fA-F0-9]{40})/);
    return m ? m[1] : v;
  };
  return (
    <div className="table-wrapper">
      <table className="table">
        <thead>
          <tr>
            <th>#</th>
            <th>Method</th>
            <th>From</th>
            <th>To</th>
            <th>Value</th>
            <th>Gas</th>
          </tr>
        </thead>
        <tbody>
          {txs.map((t) => (
            <tr key={`${t.index}-${t.tx_id}`}>
              <td>{t.index}</td>
              <td>{t.method ?? 'unknown'}</td>
              <td><AddressLink address={extractAddr(t.from)} eco={eco} /></td>
              <td><AddressLink address={extractAddr(t.to)} eco={eco} /></td>
              <td>{t.value_formatted}</td>
              <td>{t.gas_limit}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// Simple tokens table (filters out zero balances)
function TokenTable({ tokens }) {
  const rows = (tokens || []).filter((t) => t && t.raw_wei && t.raw_wei !== '0');
  if (rows.length === 0) return <div className="muted">No non-zero balances</div>;
  return (
    <div className="table-wrapper">
      <table className="table">
        <thead>
          <tr>
            <th>Token</th>
            <th>Amount</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((t) => (
            <tr key={t.token}>
              <td>{t.token}</td>
              <td>{t.formatted}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default function App() {
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [status, setStatus] = useState('idle');
  const ECOSYSTEMS = [
    { key: 'mainnet', label: 'Mainnet', file: 'output.mainnet.json' },
    { key: 'testnet', label: 'Testnet', file: 'output.testnet.json' },
    { key: 'testnet_atlas', label: 'Testnet Atlas', file: 'output.testnet_atlas.json' }
  ];
  const [eco, setEco] = useState(() => {
    try {
      return localStorage.getItem('eco') || 'mainnet';
    } catch {
      return 'mainnet';
    }
  });
  const currentEndpoint = useMemo(() => {
    const found = ECOSYSTEMS.find((e) => e.key === eco) || ECOSYSTEMS[0];
    return found.file;
  }, [eco]);

  useEffect(() => {
    let isMounted = true;

    async function load() {
      try {
        setStatus('loading');
        const response = await fetch(currentEndpoint);
        if (!response.ok) throw new Error(`Request failed with status ${response.status}`);
        const payload = await response.json();
        if (isMounted) {
          setData(payload);
          setError(null);
          setStatus('success');
        }
      } catch (err) {
        if (isMounted) {
          setError(err);
          setStatus('error');
        }
      }
    }

    load();
    const interval = setInterval(load, 60_000);
    return () => {
      isMounted = false;
      clearInterval(interval);
    };
  }, [currentEndpoint]);

  // Build id -> name map from src/chain_ids.json (which maps name -> id)
  const idToName = useMemo(() => {
    try {
      const mapping = chainIdsDoc?.chain_ids || {};
      const inv = new Map();
      for (const [name, id] of Object.entries(mapping)) {
        inv.set(Number(id), name);
      }
      return inv;
    } catch {
      return new Map();
    }
  }, []);

  const gatewayChainSet = useMemo(() => {
    const s = new Set();
    if (data?.gateway_bridgehub?.known_chains) {
      for (const id of data.gateway_bridgehub.known_chains) s.add(Number(id));
    }
    return s;
  }, [data]);

  // Map asset_id -> asset name from registered assets
  const assetIdName = useMemo(() => {
    const map = new Map();
    const l1 = data?.bridgehub?.asset_router?.registered_assets || [];
    const l2 = data?.gateway_bridgehub?.asset_router?.registered_assets || [];
    for (const a of l1) map.set(a.asset_id, a.name);
    for (const a of l2) map.set(a.asset_id, a.name);
    return map;
  }, [data]);

  // Determine newest and previous protocol versions across all chains
  const versionRanks = useMemo(() => {
    if (!data?.chains) return { latest: null, previous: null, asKey: () => '' };
    const toKey = (v) => (Array.isArray(v) && v.length === 3 ? `${v[0]}.${v[1]}.${v[2]}` : '');
    const fromKey = (k) => k.split('.').map((n) => Number(n));
    const uniq = new Set();
    for (const c of data.chains) {
      const v = c?.state_transition?.protocol_version;
      const key = toKey(v);
      if (key) uniq.add(key);
    }
    const list = Array.from(uniq);
    list.sort((a, b) => {
      const [ax, ay, az] = fromKey(a);
      const [bx, by, bz] = fromKey(b);
      if (ax !== bx) return ax - bx;
      if (ay !== by) return ay - by;
      return az - bz;
    });
    const latest = list[list.length - 1] || null;
    const previous = list.length > 1 ? list[list.length - 2] : null;
    return { latest, previous, asKey: toKey };
  }, [data]);

  const chainsBySettlement = useMemo(() => {
    if (!data?.chains) return { toGateway: [], toL1: [] };
    const toGateway = [];
    const toL1 = [];
    for (const c of data.chains) {
      if (gatewayChainSet.has(Number(c.chain_id))) toGateway.push(c);
      else toL1.push(c);
    }
    return { toGateway, toL1 };
  }, [data, gatewayChainSet]);

  return (
    <div className="app">
      <header className="app__header">
        <div className="toolbar">
          <h1>Elastic Debugger Report</h1>
          <div className="toolbar__right">
            <label className="label" htmlFor="eco">Ecosystem</label>
            <select
              id="eco"
              className="select"
              value={eco}
              onChange={(e) => {
                const v = e.target.value;
                setEco(v);
                try { localStorage.setItem('eco', v); } catch { }
              }}
            >
              {ECOSYSTEMS.map((e) => (
                <option key={e.key} value={e.key}>{e.label}</option>
              ))}
            </select>
          </div>
        </div>
        <p className="muted">Loaded from <code>{currentEndpoint}</code>. Auto-refreshes every minute.</p>
      </header>

      {status === 'loading' && (
        <section className="card placeholder">
          <div className="skeleton skeleton--title" />
          <div className="skeleton skeleton--line" />
          <div className="skeleton skeleton--line" />
        </section>
      )}

      {status === 'error' && (
        <section className="card error">
          <h2>Unable to load data</h2>
          <p>
            The dashboard could not retrieve <code>/data/output.json</code>. The file might be
            missing or the server may be unreachable. The view will keep retrying automatically.
          </p>
          <pre className="error__details">{error?.message}</pre>
        </section>
      )}

      {status === 'success' && (
        <div className="layout">
          <div className="layout__row">
            {/* L1 */}
            <NodeBox
              title="L1"
              subtitle={data?.sequencers?.l1?.sequencer?.rpc_url}
              status={data?.sequencers?.l1?.status}
              details={
                <div className="grid-2">
                  <KeyValue label="Chain ID" value={data?.sequencers?.l1?.sequencer?.chain_id} />
                  <KeyValue label="Latest block" value={data?.sequencers?.l1?.sequencer?.latest_block} />
                  <KeyValue label="Bridgehub" value={data?.bridgehub?.address} eco={eco} />
                  <KeyValue label="CTM deployer" value={data?.bridgehub?.ctm_deployer} eco={eco} />
                  <KeyValue label="Known chains" value={data?.bridgehub?.known_chains?.length ?? 0} />
                </div>
              }
            />

            {/* Gateway (if present) */}
            {data?.gateway_bridgehub && (
              <NodeBox
                title="Gateway"
                subtitle={data?.sequencers?.l2?.sequencer?.rpc_url}
                status={data?.sequencers?.l2?.status}
                arrowTo="L1"
                details={
                  <div className="grid-2">
                    <KeyValue label="Chain ID" value={data?.sequencers?.l2?.sequencer?.chain_id} />
                    <KeyValue label="Latest block" value={data?.sequencers?.l2?.sequencer?.latest_block} />
                    <KeyValue label="Bridgehub" value={data?.gateway_bridgehub?.address} eco={eco} />
                    <KeyValue label="Known chains" value={data?.gateway_bridgehub?.known_chains?.length ?? 0} />
                  </div>
                }
              />
            )}
          </div>

          <div className="layout__row">
            <div className="column">
              <h2 className="section-title">Chains settling to Gateway</h2>
              <div className="grid">
                {chainsBySettlement.toGateway.map((c) => {
                  const st = c.state_transition;
                  const name = idToName.get(Number(c.chain_id));
                  const key = versionRanks.asKey(st?.protocol_version);
                  const batchUpdatedRelative = formatRelativeToRefresh(
                    Number(st?.last_batch_update_unix),
                    Number(data?.generated_at_unix)
                  );
                  const batchUpdatedTitle = formatTimestampTitle(Number(st?.last_batch_update_unix));
                  const verTone = key
                    ? key === versionRanks.latest
                      ? 'ok'
                      : key === versionRanks.previous
                        ? 'warn'
                        : 'danger'
                    : 'neutral';
                  return (
                    <section className="chain card" key={c.chain_id}>
                      <header className="chain__header">
                        <div className="row-left">
                          <h3 className="chain__title">Chain {c.chain_id}{name ? ` · ${name}` : ''}</h3>
                          <span className="settlement">→ Gateway</span>
                        </div>
                        <div className="row-right">
                          <Badge tone={st ? 'success' : 'danger'}>{st ? 'HEALTHY' : 'ERROR'}</Badge>
                        </div>
                      </header>
                      {st ? (
                        <>
                          <div className="stats">
                            <div className={`pill ${verTone === 'ok' ? 'pill--ok' : verTone === 'warn' ? 'pill--warn' : verTone === 'danger' ? 'pill--danger' : ''}`}>
                              Protocol {formatVersion(st.protocol_version)}
                            </div>
                            <div className="pill">Batches C/V/E {st.total_batches_committed}/{st.total_batches_verified}/{st.total_batches_executed}</div>
                            {batchUpdatedRelative && (
                              <div className="pill" title={batchUpdatedTitle ?? undefined}>
                                Last batch update {batchUpdatedRelative}
                              </div>
                            )}
                            <div className="pill">Queue {st.queue.unprocessed}/{st.queue.total}</div>
                            {typeof c.priority_tree_verified === 'boolean' && (
                              <div className={`pill ${c.priority_tree_verified ? 'pill--ok' : 'pill--warn'}`}>
                                Priority root {c.priority_tree_verified ? 'VALID' : 'INVALID'}
                              </div>
                            )}
                          </div>
                          <div className="grid-2">
                            <KeyValue label="Hyperchain" value={st.hyperchain} eco={eco} />
                            <KeyValue label="Verifier" value={st.verifier} eco={eco} />
                            <KeyValue label="Admin" value={st.admin} eco={eco} />
                            <KeyValue label="Settlement layer" value={st.settlement_layer} eco={eco} />
                            <KeyValue label="Validator timelock post-v29" value={c.validator_timelock_post_v29} eco={eco} />
                            {st.base_token_asset_id && assetIdName.get(st.base_token_asset_id) && (
                              <KeyValue label="Base token" value={assetIdName.get(st.base_token_asset_id)} />
                            )}
                          </div>
                          <Collapsible title="Posting accounts" count={(c.commit_posters?.length || 0) + (c.proof_posters?.length || 0)}>
                            {c.posting_accounts_error ? (
                              <div className="muted">{c.posting_accounts_error}</div>
                            ) : (
                              <div className="grid-2">
                                <AddressList label="Commit posters" addresses={c.commit_posters} eco={eco} />
                                <AddressList label="Proof posters" addresses={c.proof_posters} eco={eco} />
                              </div>
                            )}
                          </Collapsible>
                          {(() => {
                            const bal = (data?.l1_balances || []).find((b) => Number(b.chain_id) === Number(c.chain_id));
                            const tokens = (bal?.tokens || []).filter((t) => t && t.raw_wei && t.raw_wei !== '0');
                            return (
                              <Collapsible title="Tokens" count={tokens.length}>
                                <TokenTable tokens={tokens} />
                              </Collapsible>
                            );
                          })()}
                          <Collapsible title="Priority transactions" count={c.priority_transactions?.length || 0}>
                            <PriorityTable txs={c.priority_transactions} eco={eco} />
                          </Collapsible>
                        </>
                      ) : (
                        <div className="muted">{c.state_transition_error ?? 'State transition unavailable'}</div>
                      )}
                    </section>
                  );
                })}
                {chainsBySettlement.toGateway.length === 0 && (
                  <div className="muted">No chains registered on Gateway</div>
                )}
              </div>
            </div>

            <div className="column">
              <h2 className="section-title">Chains settling to L1</h2>
              <div className="grid">
                {chainsBySettlement.toL1.map((c) => {
                  const st = c.state_transition;
                  const name = idToName.get(Number(c.chain_id));
                  const key = versionRanks.asKey(st?.protocol_version);
                  const batchUpdatedRelative = formatRelativeToRefresh(
                    Number(st?.last_batch_update_unix),
                    Number(data?.generated_at_unix)
                  );
                  const batchUpdatedTitle = formatTimestampTitle(Number(st?.last_batch_update_unix));
                  const verTone = key
                    ? key === versionRanks.latest
                      ? 'ok'
                      : key === versionRanks.previous
                        ? 'warn'
                        : 'danger'
                    : 'neutral';
                  return (
                    <section className="chain card" key={c.chain_id}>
                      <header className="chain__header">
                        <div className="row-left">
                          <h3 className="chain__title">Chain {c.chain_id}{name ? ` · ${name}` : ''}</h3>
                          <span className="settlement">→ L1</span>
                        </div>
                        <div className="row-right">
                          <Badge tone={st ? 'success' : 'danger'}>{st ? 'HEALTHY' : 'ERROR'}</Badge>
                        </div>
                      </header>
                      {st ? (
                        <>
                          <div className="stats">
                            <div className={`pill ${verTone === 'ok' ? 'pill--ok' : verTone === 'warn' ? 'pill--warn' : verTone === 'danger' ? 'pill--danger' : ''}`}>
                              Protocol {formatVersion(st.protocol_version)}
                            </div>
                            <div className="pill">Batches C/V/E {st.total_batches_committed}/{st.total_batches_verified}/{st.total_batches_executed}</div>
                            {batchUpdatedRelative && (
                              <div className="pill" title={batchUpdatedTitle ?? undefined}>
                                Last batch update {batchUpdatedRelative}
                              </div>
                            )}
                            <div className="pill">Queue {st.queue.unprocessed}/{st.queue.total}</div>
                            {typeof c.priority_tree_verified === 'boolean' && (
                              <div className={`pill ${c.priority_tree_verified ? 'pill--ok' : 'pill--warn'}`}>
                                Priority root {c.priority_tree_verified ? 'VALID' : 'INVALID'}
                              </div>
                            )}
                          </div>
                          <div className="grid-2">
                            <KeyValue label="Hyperchain" value={st.hyperchain} eco={eco} />
                            <KeyValue label="Verifier" value={st.verifier} eco={eco} />
                            <KeyValue label="Admin" value={st.admin} eco={eco} />
                            <KeyValue label="Settlement layer" value={st.settlement_layer} eco={eco} />
                            <KeyValue label="Validator timelock post-v29" value={c.validator_timelock_post_v29} eco={eco} />
                            {st.base_token_asset_id && assetIdName.get(st.base_token_asset_id) && (
                              <KeyValue label="Base token" value={assetIdName.get(st.base_token_asset_id)} />
                            )}
                          </div>
                          <Collapsible title="Posting accounts" count={(c.commit_posters?.length || 0) + (c.proof_posters?.length || 0)}>
                            {c.posting_accounts_error ? (
                              <div className="muted">{c.posting_accounts_error}</div>
                            ) : (
                              <div className="grid-2">
                                <AddressList label="Commit posters" addresses={c.commit_posters} eco={eco} />
                                <AddressList label="Proof posters" addresses={c.proof_posters} eco={eco} />
                              </div>
                            )}
                          </Collapsible>
                          {(() => {
                            const bal = (data?.l1_balances || []).find((b) => Number(b.chain_id) === Number(c.chain_id));
                            const tokens = (bal?.tokens || []).filter((t) => t && t.raw_wei && t.raw_wei !== '0');
                            return (
                              <Collapsible title="Tokens" count={tokens.length}>
                                <TokenTable tokens={tokens} />
                              </Collapsible>
                            );
                          })()}
                          <Collapsible title="Priority transactions" count={c.priority_transactions?.length || 0}>
                            <PriorityTable txs={c.priority_transactions} eco={eco} />
                          </Collapsible>
                        </>
                      ) : (
                        <div className="muted">{c.state_transition_error ?? 'State transition unavailable'}</div>
                      )}
                    </section>
                  );
                })}
                {chainsBySettlement.toL1.length === 0 && (
                  <div className="muted">No chains registered on L1</div>
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
