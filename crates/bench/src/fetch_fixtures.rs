/// Download a range of mainnet blocks and persist them to crates/bench/fixtures/.
///
/// Layout after a successful run:
///   fixtures/headers.bin           — N × 80-byte raw headers in height order
///   fixtures/blocks/NNNNNN.bin     — raw P2P wire bytes for each block
///
/// Re-running is idempotent: blocks already on disk are skipped.
///
/// Usage:
///   # Default: blocks 1–1000
///   cargo run --release --bin fetch-fixtures
///
///   # Named preset (locator hash looked up automatically)
///   cargo run --release --bin fetch-fixtures -- --preset ordinals
///   cargo run --release --bin fetch-fixtures -- --preset runes
///
///   # Explicit range with a known locator hash one step before --start
///   cargo run --release --bin fetch-fixtures -- \
///     --start 767430 --end 768430 \
///     --locator 00000000000000000002a7c4c1e48d76c5a37902165a270156b7a8d72728a054
///
/// Built-in presets:
///   early    blocks      1–1000  (genesis era, tiny blocks)
///   ordinals blocks 767430–768430 (Ordinals inscription era, Dec 2022)
///   runes    blocks 840000–841000 (Runes launch / halving, Apr 2024)
use std::collections::HashMap;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mantle::codec::Encode;
use mantle::network::{
    Command, GetHeadersMsg, HeadersMsg, InvEntry, InvMsg, InvType, Network, NetworkAddr,
    VersionMsg, services,
};
use mantle::peer::{InboundMsg, Node, OutboundMsg, PeerId};
use mantle::primitives::{BlockHeader, Hash256};

/// fetch-fixtures only ever pulls mainnet blocks (the presets are mainnet
/// heights). The CLI does not yet expose a `--network` flag — keep the
/// constant so the seed list, magic, and genesis hash come from one source.
const NETWORK: Network = Network::Mainnet;

/// Maximum blocks in-flight per peer at once.
const PEER_WINDOW: usize = 8;
/// Seconds without progress before a peer's in-flight blocks are reclaimed.
const STALL_SECS: u64 = 30;

// ── Presets ───────────────────────────────────────────────────────────────────

struct Preset {
    name: &'static str,
    start: u32,
    end: u32,
    locator: &'static str,
}

const PRESETS: &[Preset] = &[
    Preset {
        name: "early",
        start: 1,
        end: 1000,
        locator: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
    },
    Preset {
        name: "ordinals",
        start: 767_430,
        end: 768_430,
        locator: "00000000000000000002a7c4c1e48d76c5a37902165a270156b7a8d72728a054",
    },
    Preset {
        name: "runes",
        start: 840_000,
        end: 841_000,
        locator: "0000000000000000000172014ba58d66455762add0512355ad651207918494ab",
    },
];

fn find_preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

// ── CLI ───────────────────────────────────────────────────────────────────────

struct Args {
    preset: String,
    start: u32,
    end: u32,
    locator: Hash256,
    /// Stop after fetching this many blocks (0 = no limit).
    limit: usize,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;

    let mut start: Option<u32> = None;
    let mut end: Option<u32> = None;
    let mut locator_hex: Option<String> = None;
    let mut preset_name: Option<String> = None;
    let mut limit: usize = 0;

    while i < raw.len() {
        match raw[i].as_str() {
            "--preset" => {
                i += 1;
                preset_name = Some(raw[i].clone());
            }
            "--start" => {
                i += 1;
                start = Some(raw[i].parse().expect("--start must be a number"));
            }
            "--end" => {
                i += 1;
                end = Some(raw[i].parse().expect("--end must be a number"));
            }
            "--locator" => {
                i += 1;
                locator_hex = Some(raw[i].clone());
            }
            "--limit" => {
                i += 1;
                limit = raw[i].parse().expect("--limit must be a number");
            }
            other => {
                eprintln!("Unknown argument: {other}");
                eprintln!(
                    "Usage: fetch-fixtures [--preset NAME | --start N --end N --locator HASH] [--limit N]"
                );
                std::process::exit(1);
            }
        }
        i += 1;
    }

    if let Some(name) = preset_name {
        let p = find_preset(&name).unwrap_or_else(|| {
            let names: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
            eprintln!("Unknown preset '{name}'. Available: {}", names.join(", "));
            std::process::exit(1);
        });
        return Args {
            preset: p.name.to_string(),
            start: p.start,
            end: p.end,
            locator: Hash256::from_display_hex(p.locator).expect("hardcoded locator"),
            limit,
        };
    }

    if start.is_none() && end.is_none() && locator_hex.is_none() {
        let p = find_preset("early").expect("early preset exists");
        return Args {
            preset: p.name.to_string(),
            start: p.start,
            end: p.end,
            locator: Hash256::from_display_hex(p.locator).expect("hardcoded locator"),
            limit,
        };
    }

    let start = start.expect("--start required when not using --preset");
    let end = end.expect("--end required when not using --preset");
    let hex = locator_hex.expect("--locator required when not using --preset");
    let locator = Hash256::from_display_hex(&hex).unwrap_or_else(|_| {
        eprintln!("Invalid --locator hash");
        std::process::exit(1);
    });

    assert!(start <= end, "--start must be <= --end");
    assert!(
        end - start <= 2000,
        "range must be ≤ 2000 blocks (one getheaders response)"
    );

    Args {
        preset: format!("custom_{start}_{end}"),
        start,
        end,
        locator,
        limit,
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

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
        version: 70015,
        services: services::NODE_NETWORK,
        timestamp: now,
        receiver: empty,
        sender: empty,
        nonce: 0xdead_beef_cafe_5678,
        user_agent: "/mantle-fetch:0.1/".to_string(),
        start_height: 0,
        relay: false,
    }
}

fn fixtures_dir(preset: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(preset)
}

fn block_path(dir: &Path, height: u32) -> PathBuf {
    dir.join("blocks").join(format!("{height:07}.bin"))
}

fn encode_getdata(hashes: &[Hash256]) -> Vec<u8> {
    let entries: Vec<InvEntry> = hashes
        .iter()
        .map(|&hash| InvEntry {
            kind: InvType::WitnessBlock,
            hash,
        })
        .collect();
    let mut buf = Vec::new();
    InvMsg(entries)
        .encode(&mut buf)
        .expect("Vec write cannot fail");
    buf
}

fn encode_getheaders(locator: Hash256) -> Vec<u8> {
    GetHeadersMsg::single_locator(locator).encode_to_vec()
}

fn frame_and_send(node: &Node, peer: PeerId, fd: i32, cmd: Command, payload: &[u8]) {
    let mut framed = Vec::new();
    NETWORK
        .magic()
        .frame(cmd, payload, &mut framed)
        .expect("frame cannot fail");
    let _ = node.outbound_tx.push(OutboundMsg {
        peer,
        fd,
        bytes: framed.into_boxed_slice(),
    });
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args = parse_args();
    let n = args.end - args.start + 1;

    let fixtures = fixtures_dir(&args.preset);
    std::fs::create_dir_all(fixtures.join("blocks")).expect("create fixtures/blocks");

    let mut have: Vec<bool> = (args.start..=args.end)
        .map(|h| block_path(&fixtures, h).exists())
        .collect();

    let need_count = have.iter().filter(|&&x| !x).count();
    if need_count == 0 {
        std::fs::write(fixtures.join("metadata.bin"), args.start.to_le_bytes())
            .expect("write metadata.bin");
        println!(
            "All {n} blocks ({}-{}) already present. Nothing to do.",
            args.start, args.end
        );
        return;
    }
    println!(
        "{need_count}/{n} blocks missing ({}-{}) — connecting to mainnet...",
        args.start, args.end
    );

    match run(&fixtures, &args, &mut have) {
        Ok(true) => println!("Done."),
        Ok(false) => {
            eprintln!("Fetch incomplete.");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Fatal error: {e}");
            std::process::exit(1);
        }
    }
}

// ── Per-peer state ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum PeerPhase {
    WaitingHandshake,
    WaitingHeaders,
    Ready,
    Dead,
}

struct PeerSlot {
    id: PeerId,
    fd: i32,
    phase: PeerPhase,
    /// Blocks assigned to this peer: (hash, `time_assigned`).
    in_flight: Vec<(Hash256, Instant)>,
}

// ── run ───────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn run(fixtures: &Path, args: &Args, have: &mut [bool]) -> Result<bool, mantle::peer::Error> {
    let node = Node::new(NETWORK.magic(), our_version())?;
    let inbound = node.inbound_rx.clone();

    let seeds = NETWORK.default_seeds();
    let mut peers: Vec<PeerSlot> = Vec::with_capacity(seeds.len());
    for addr_str in seeds {
        let addr: SocketAddrV4 = addr_str.parse().expect("valid addr");
        match node.connect(addr) {
            Ok((id, fd)) => {
                eprintln!("  Dialing {addr} → peer {id:?}");
                peers.push(PeerSlot {
                    id,
                    fd,
                    phase: PeerPhase::WaitingHandshake,
                    in_flight: Vec::new(),
                });
            }
            Err(e) => eprintln!("  connect {addr}: {e}"),
        }
    }
    if peers.is_empty() {
        eprintln!("No peers could be dialed.");
        return Ok(false);
    }

    let mut headers_peer_elected = false;
    let mut need_heights: Vec<(u32, Hash256)> = Vec::new();
    // Mapping hash → height for received blocks.
    let mut pending: HashMap<Hash256, u32> = HashMap::new();
    let mut all_headers: Vec<(u32, BlockHeader)> = Vec::new();

    // Sequential cursor: next index in need_heights to assign.
    let mut next_assign = 0usize;
    // Blocks reclaimed from stalled peers, to be reassigned before next_assign advances.
    let mut retry: Vec<Hash256> = Vec::new();

    let mut saved = 0u32;
    let deadline = Instant::now() + Duration::from_secs(120);

    loop {
        // Block until the RX thread delivers the next message or a Tick.
        let Some(msg) = inbound.pop_block() else {
            break;
        };

        match msg {
            InboundMsg::Tick => {
                if Instant::now() > deadline {
                    eprintln!("  timeout");
                    return Ok(false);
                }

                // Reclaim blocks from stalled peers.
                let stall = Duration::from_secs(STALL_SECS);
                for peer in &mut peers {
                    if peer.phase != PeerPhase::Ready {
                        continue;
                    }
                    let stalled: Vec<Hash256> = peer
                        .in_flight
                        .iter()
                        .filter(|(_, t)| t.elapsed() > stall)
                        .map(|(h, _)| *h)
                        .collect();
                    if !stalled.is_empty() {
                        eprintln!(
                            "  peer {:?}: {} blocks stalled — reclaiming",
                            peer.id,
                            stalled.len()
                        );
                        peer.in_flight.retain(|(h, _)| !stalled.contains(h));
                        retry.extend(stalled);
                    }
                }

                // Refill any peer that has capacity after stall reclaim.
                for i in 0..peers.len() {
                    if peers[i].phase == PeerPhase::Ready && peers[i].in_flight.len() < PEER_WINDOW
                    {
                        topup(
                            &node,
                            &mut peers,
                            i,
                            &need_heights,
                            &mut next_assign,
                            &mut retry,
                        );
                    }
                }

                let all_dead = peers.iter().all(|p| p.phase == PeerPhase::Dead);
                if all_dead {
                    eprintln!("  All peers dead before completion.");
                    return Ok(false);
                }
            }

            InboundMsg::Network {
                peer: peer_id,
                command,
                payload,
            } => {
                let Some(idx) = peers.iter().position(|p| p.id == peer_id) else {
                    continue;
                };
                let cmd = Command::from_command_name(mantle::network::CommandName(command));

                // First application message from this peer → handshake done.
                if peers[idx].phase == PeerPhase::WaitingHandshake {
                    eprintln!("  peer {:?} handshake complete", peers[idx].id);

                    if headers_peer_elected {
                        peers[idx].phase = PeerPhase::Ready;
                        if !need_heights.is_empty() {
                            topup(
                                &node,
                                &mut peers,
                                idx,
                                &need_heights,
                                &mut next_assign,
                                &mut retry,
                            );
                        }
                    } else {
                        headers_peer_elected = true;
                        peers[idx].phase = PeerPhase::WaitingHeaders;
                        let (pid, fd) = (peers[idx].id, peers[idx].fd);
                        eprintln!(
                            "  peer {pid:?} → getheaders (locator {})",
                            args.locator.to_display_hex()
                        );
                        frame_and_send(
                            &node,
                            pid,
                            fd,
                            Command::GetHeaders,
                            &encode_getheaders(args.locator),
                        );
                    }
                    continue;
                }

                match cmd {
                    Some(Command::Headers) if peers[idx].phase == PeerPhase::WaitingHeaders => {
                        peers[idx].phase = PeerPhase::Ready;

                        let mut headers: Vec<BlockHeader> = Vec::new();
                        HeadersMsg::decode_into_limited::<2_000>(
                            &mut std::io::Cursor::new(&payload),
                            &mut headers,
                        )
                        .map_err(mantle::peer::Error::Codec)?;

                        for (i, hdr) in headers.iter().enumerate() {
                            let height = args.start + u32::try_from(i).expect("i < 2000");
                            if height > args.end {
                                break;
                            }
                            all_headers.push((height, *hdr));
                            let slot = (height - args.start) as usize;
                            if !have[slot] {
                                let hash = hdr.hash();
                                need_heights.push((height, hash));
                                pending.insert(hash, height);
                            }
                        }

                        if need_heights.is_empty() {
                            eprintln!("  No blocks needed (all already on disk).");
                            write_headers_bin(fixtures, &all_headers, args);
                            return Ok(true);
                        }

                        let ready_count =
                            peers.iter().filter(|p| p.phase == PeerPhase::Ready).count();
                        eprintln!(
                            "  {} headers, {} blocks needed — {} peers ready",
                            all_headers.len(),
                            need_heights.len(),
                            ready_count
                        );

                        // Fill the window for every ready peer.
                        let ready: Vec<usize> = (0..peers.len())
                            .filter(|&i| peers[i].phase == PeerPhase::Ready)
                            .collect();
                        for peer_idx in ready {
                            topup(
                                &node,
                                &mut peers,
                                peer_idx,
                                &need_heights,
                                &mut next_assign,
                                &mut retry,
                            );
                        }
                    }

                    Some(Command::Block) => {
                        let msg_ref = &payload;
                        if let Some(height) = handle_block(msg_ref, &pending, fixtures)? {
                            have[(height - args.start) as usize] = true;
                            saved += 1;
                            if let Some(pos) = peers[idx]
                                .in_flight
                                .iter()
                                .position(|(h, _)| pending.get(h) == Some(&height))
                            {
                                peers[idx].in_flight.swap_remove(pos);
                            }

                            let total = u32::try_from(need_heights.len()).expect("≤2001");
                            eprintln!(
                                "  [{saved}/{total}] block {height} ← peer {:?}  (in-flight: {})",
                                peers[idx].id,
                                peers[idx].in_flight.len()
                            );

                            let limit_hit = args.limit > 0
                                && usize::try_from(saved).unwrap_or(usize::MAX) >= args.limit;
                            if saved == total || limit_hit {
                                write_headers_bin(fixtures, &all_headers, args);
                                return Ok(true);
                            }

                            topup(
                                &node,
                                &mut peers,
                                idx,
                                &need_heights,
                                &mut next_assign,
                                &mut retry,
                            );
                        }
                    }

                    _ => {}
                }
            }
        }
    }
    Ok(false)
}

/// Top up peer `peer_idx` to `PEER_WINDOW` in-flight blocks.
/// Drains `retry` first, then advances `next_assign` into `need_heights`.
fn topup(
    node: &Node,
    peers: &mut [PeerSlot],
    peer_idx: usize,
    need_heights: &[(u32, Hash256)],
    next_assign: &mut usize,
    retry: &mut Vec<Hash256>,
) {
    let want = PEER_WINDOW.saturating_sub(peers[peer_idx].in_flight.len());
    if want == 0 {
        return;
    }

    let mut hashes: Vec<Hash256> = Vec::with_capacity(want);

    // Drain retries first.
    while hashes.len() < want && !retry.is_empty() {
        hashes.push(retry.swap_remove(0));
    }

    // Then advance the sequential cursor.
    while hashes.len() < want && *next_assign < need_heights.len() {
        hashes.push(need_heights[*next_assign].1);
        *next_assign += 1;
    }

    if hashes.is_empty() {
        return;
    }

    let (pid, fd) = (peers[peer_idx].id, peers[peer_idx].fd);
    eprintln!("  → peer {pid:?}: getdata {} blocks", hashes.len());
    frame_and_send(node, pid, fd, Command::GetData, &encode_getdata(&hashes));

    let now = Instant::now();
    peers[peer_idx]
        .in_flight
        .extend(hashes.iter().map(|&h| (h, now)));
}

fn handle_block(
    payload: &[u8],
    pending: &HashMap<Hash256, u32>,
    fixtures: &Path,
) -> Result<Option<u32>, mantle::peer::Error> {
    if payload.len() < 80 {
        return Ok(None);
    }
    let hdr = BlockHeader::from_bytes(payload[..80].try_into().expect("slice is 80 bytes"))
        .map_err(mantle::peer::Error::Codec)?;
    let hash = hdr.hash();

    let Some(&height) = pending.get(&hash) else {
        return Ok(None);
    };

    let path = block_path(fixtures, height);
    std::fs::write(&path, payload).map_err(mantle::peer::Error::Codec)?;

    Ok(Some(height))
}

fn write_headers_bin(fixtures: &Path, headers: &[(u32, BlockHeader)], args: &Args) {
    let mut sorted = headers.to_vec();
    sorted.sort_by_key(|(h, _)| *h);

    let path = fixtures.join("headers.bin");
    let mut out = Vec::with_capacity(sorted.len() * 80);
    for (_, hdr) in &sorted {
        out.extend_from_slice(&hdr.to_bytes());
    }
    std::fs::write(&path, &out).expect("write headers.bin");

    std::fs::write(fixtures.join("metadata.bin"), args.start.to_le_bytes())
        .expect("write metadata.bin");

    println!(
        "Wrote {} headers (heights {}–{}) to {}",
        sorted.len(),
        args.start,
        args.end,
        path.display()
    );
}
