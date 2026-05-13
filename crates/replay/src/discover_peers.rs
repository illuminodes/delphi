//! `discover-peers` — standalone P2P peer-list harvester.
//!
//! Dials a network's DNS seeds (or `--peer` overrides), runs the version
//! handshake, signals BIP155 support via `sendaddrv2`, asks every connected
//! peer for its peer table via `getaddr`, and accumulates the IPv4
//! [`AddrEntry`] / [`AddrV2Entry`] responses. Writes a deduped `IP:PORT`
//! list to disk; intended as a one-shot bootstrap helper so later replay
//! runs have a usable static peer list without depending on DNS-seeder
//! daemons (which often don't speak the full P2P protocol).
//!
//! Usage:
//!     cargo run --release --bin discover-peers -- \
//!         --network testnet4 \
//!         --out crates/bench/fixtures/testnet4/peers.txt \
//!         [--peer HOST:PORT]…  [--seconds 60]
//!
//! The binary deliberately doesn't share state with `mantle-replay`: it
//! lives in the same crate only because it reuses the [`mantle::peer::Node`]
//! handle and the [`mantle::network`] codecs.

use std::collections::BTreeSet;
use std::io::Cursor;
use std::net::{SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mantle::network::{
    AddrMsg, AddrV2Addr, AddrV2Msg, Command, CommandName, Network, NetworkAddr, VersionMsg,
    services,
};
use mantle::peer::{InboundMsg, Node, OutboundMsg, PeerId};

const HELP: &str = "\
discover-peers — collect a static peer list by talking to DNS seeds.

Usage:
    discover-peers [OPTIONS]

Options:
    --network NAME        mainnet | testnet3 | testnet4 | signet | regtest
                          (default: testnet4).
    --peer HOST:PORT      Extra P2P peer to dial; may be repeated. Combined
                          with the network's default DNS seeds.
    --out FILE            Output path for the deduped IP:PORT list
                          (default: ./peers.txt).
    --seconds N           Wall-clock budget for the discovery loop
                          (default: 60).
    --help                Print this message and exit.
";

struct Args {
    network: Network,
    extra_peers: Vec<SocketAddrV4>,
    out: PathBuf,
    seconds: u64,
}

fn parse_args() -> Result<Args, String> {
    let mut network = Network::Testnet4;
    let mut extra_peers: Vec<SocketAddrV4> = Vec::new();
    let mut out = PathBuf::from("./peers.txt");
    let mut seconds: u64 = 60;

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--network" => {
                i += 1;
                let v = raw.get(i).ok_or("--network requires a name")?;
                network = Network::parse(v).ok_or_else(|| format!("unknown network: {v}"))?;
            }
            "--peer" => {
                i += 1;
                let v = raw.get(i).ok_or("--peer requires HOST:PORT")?;
                let addr: SocketAddrV4 =
                    v.parse().map_err(|e| format!("invalid --peer {v}: {e}"))?;
                extra_peers.push(addr);
            }
            "--out" => {
                i += 1;
                let v = raw.get(i).ok_or("--out requires a path")?;
                out = PathBuf::from(v);
            }
            "--seconds" => {
                i += 1;
                let v = raw.get(i).ok_or("--seconds requires N")?;
                seconds = v
                    .parse()
                    .map_err(|e| format!("invalid --seconds {v}: {e}"))?;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }

    Ok(Args {
        network,
        extra_peers,
        out,
        seconds,
    })
}

fn our_version() -> VersionMsg {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs(),
    )
    .unwrap_or(i64::MAX);
    let empty = NetworkAddr {
        services: 0,
        ip: [0u8; 16],
        port: 0,
    };
    VersionMsg {
        version: 70016, // ≥70016 advertises BIP155 readiness
        services: services::NODE_NETWORK,
        timestamp: now,
        receiver: empty,
        sender: empty,
        nonce: 0xfeed_face_dead_beef,
        user_agent: "/mantle-discover-peers:0.1.0/".to_string(),
        start_height: 0,
        relay: false,
    }
}

/// Resolve DNS-seed `host:port` strings + `--peer` extras to a flat list of
/// dialable IPv4 addresses. Same shape as `mantle-replay`, kept inline here
/// so the discovery binary is self-contained.
fn resolve_peers(network: Network, extras: &[SocketAddrV4]) -> Vec<SocketAddrV4> {
    let mut out: Vec<SocketAddrV4> = extras.to_vec();
    for seed in network.default_seeds() {
        if let Ok(addr) = seed.parse::<SocketAddrV4>() {
            out.push(addr);
            continue;
        }
        match seed.to_socket_addrs() {
            Ok(iter) => {
                let before = out.len();
                for addr in iter {
                    if let SocketAddr::V4(v4) = addr {
                        out.push(v4);
                    }
                }
                let resolved = out.len() - before;
                if resolved == 0 {
                    eprintln!("  seed {seed} resolved to 0 IPv4 addresses (skipped)");
                } else {
                    eprintln!("  seed {seed} resolved to {resolved} address(es)");
                }
            }
            Err(e) => eprintln!("  failed to resolve seed {seed}: {e}"),
        }
    }
    out
}

fn frame_and_send(node: &Node, network: Network, peer: PeerId, fd: i32, cmd: Command) {
    let mut framed = Vec::new();
    network
        .magic()
        .frame(cmd, &[], &mut framed)
        .expect("frame cannot fail");
    let _ = node.outbound_tx.push(OutboundMsg {
        peer,
        fd,
        bytes: framed.into_boxed_slice(),
    });
}

struct PeerState {
    /// Have we sent `sendaddrv2` + `getaddr` to this peer yet?
    asked: bool,
    /// The address we dialed for this peer. Recorded so a peer that completes
    /// the version handshake but never gossips an `addr`/`addrv2` (Bitcoin
    /// Core ≥0.21 only honours `getaddr` from peers *it* dialed, not from
    /// peers that dialed it) still ends up in the harvest by virtue of
    /// having proven itself reachable.
    dialed: SocketAddrV4,
    /// Have we observed any post-handshake message from this peer?
    handshook: bool,
}

#[allow(clippy::too_many_lines)]
fn run(args: &Args) -> Result<(), String> {
    eprintln!(
        "discover-peers: network={} budget={}s out={}",
        args.network.name(),
        args.seconds,
        args.out.display()
    );

    let peer_addrs = resolve_peers(args.network, &args.extra_peers);
    if peer_addrs.is_empty() {
        return Err(format!(
            "no peers: {} has no reachable seeds; supply --peer HOST:PORT",
            args.network.name()
        ));
    }
    eprintln!("  dialing {} candidate peer(s)", peer_addrs.len());

    let mut node =
        Node::new(args.network.magic(), our_version()).map_err(|e| format!("Node::new: {e}"))?;

    // Map PeerId → fd + state. We don't care about per-peer sequencing;
    // each peer independently sends us its addr table.
    let mut peers: std::collections::HashMap<PeerId, (i32, PeerState)> =
        std::collections::HashMap::new();
    for addr in &peer_addrs {
        match node.connect(*addr) {
            Ok((id, fd)) => {
                peers.insert(
                    id,
                    (
                        fd,
                        PeerState {
                            asked: false,
                            dialed: *addr,
                            handshook: false,
                        },
                    ),
                );
            }
            Err(e) => eprintln!("  connect {addr}: {e}"),
        }
    }
    if peers.is_empty() {
        return Err("no peers accepted the connect call".into());
    }

    // Harvest. Dedup by SocketAddrV4 to make output stable.
    let mut harvest: BTreeSet<SocketAddrV4> = BTreeSet::new();
    let inbound = node.inbound_rx.clone();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(args.seconds);
    let mut last_log = started;

    while Instant::now() < deadline {
        let Some(msg) = inbound.pop_block() else {
            // Channel closed — RX thread gone.
            eprintln!("  inbound channel closed; stopping early");
            break;
        };

        if last_log.elapsed() >= Duration::from_secs(5) {
            eprintln!("  …{} unique IPv4 peers harvested so far", harvest.len());
            last_log = Instant::now();
        }

        match msg {
            InboundMsg::Tick => {
                // Belt-and-suspenders: 8 seconds after dialing, force-send
                // sendaddrv2+getaddr to any peer we haven't asked yet —
                // some peers don't volunteer post-handshake messages, so
                // waiting for "first non-handshake message" deadlocks.
                if started.elapsed() >= Duration::from_secs(8) {
                    for (peer, (fd, state)) in &mut peers {
                        if !state.asked {
                            state.asked = true;
                            frame_and_send(&node, args.network, *peer, *fd, Command::SendAddrV2);
                            frame_and_send(&node, args.network, *peer, *fd, Command::GetAddr);
                        }
                    }
                }
            }
            InboundMsg::Network {
                peer,
                command,
                payload,
            } => {
                let cmd = Command::from_command_name(CommandName(command));

                // Mark the peer as handshook on its first network message —
                // having proven reachable + protocol-speaking, the peer is
                // a viable harvest entry even if it never gossips an addr.
                if let Some((fd, state)) = peers.get_mut(&peer) {
                    state.handshook = true;
                    if !state.asked {
                        let fd_copy = *fd;
                        state.asked = true;
                        frame_and_send(&node, args.network, peer, fd_copy, Command::SendAddrV2);
                        frame_and_send(&node, args.network, peer, fd_copy, Command::GetAddr);
                    }
                }

                match cmd {
                    Some(Command::Addr) => {
                        match AddrMsg::decode_limited(&mut Cursor::new(&payload)) {
                            Ok(msg) => {
                                for entry in msg.0 {
                                    // Only IPv4-mapped entries are useful — IPv6
                                    // works fine over the network but our existing
                                    // peer code is v4-only.
                                    if entry.ip[..12] == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff]
                                    {
                                        let v4 = std::net::Ipv4Addr::new(
                                            entry.ip[12],
                                            entry.ip[13],
                                            entry.ip[14],
                                            entry.ip[15],
                                        );
                                        harvest.insert(SocketAddrV4::new(v4, entry.port));
                                    }
                                }
                            }
                            Err(e) => eprintln!("  peer {peer:?} sent malformed addr: {e}"),
                        }
                    }
                    Some(Command::AddrV2) => {
                        match AddrV2Msg::decode_limited(&mut Cursor::new(&payload)) {
                            Ok(msg) => {
                                for entry in msg.0 {
                                    if let AddrV2Addr::Ipv4(_) = entry.addr
                                        && let Some(v4) = entry.ip_v4()
                                    {
                                        harvest.insert(v4);
                                    }
                                }
                            }
                            Err(e) => eprintln!("  peer {peer:?} sent malformed addrv2: {e}"),
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Fold in every peer that completed handshake. Bitcoin Core ≥0.21 only
    // honours `getaddr` from peers it dialed (we're inbound from its
    // perspective when we dialed it), so a peer typically won't gossip its
    // addrman to us — but if we got past version handshake, the IP itself
    // is known-reachable and known-bitcoin-speaking, which is exactly what a
    // bootstrap peer list needs.
    let handshook_count = peers.values().filter(|(_, s)| s.handshook).count();
    for (_, state) in peers.values() {
        if state.handshook {
            harvest.insert(state.dialed);
        }
    }
    eprintln!(
        "  {handshook_count} of {} dialed peers completed handshake; harvest now {}",
        peer_addrs.len(),
        harvest.len()
    );
    eprintln!("  collected {} unique IPv4 peers", harvest.len());
    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return Err(format!("create_dir_all {}: {e}", parent.display()));
    }
    let mut text = String::new();
    for addr in &harvest {
        text.push_str(&addr.to_string());
        text.push('\n');
    }
    std::fs::write(&args.out, text).map_err(|e| format!("write {}: {e}", args.out.display()))?;
    eprintln!("  wrote {}", args.out.display());

    node.shutdown();
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fatal: {e}");
            ExitCode::FAILURE
        }
    }
}
