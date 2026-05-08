#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use std::net::SocketAddrV4;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mantle::codec::Encode;
use mantle::network::{
    Command, GetHeadersMsg, HeadersMsg, Magic, NetworkAddr, VersionMsg, services,
};
use mantle::peer::{HandshakeState, InboundMsg, Node};
use mantle::primitives::Hash256;

// Resolved from multiple DNS seeds — tried in order until one responds.
const SEED_PEERS: &[&str] = &[
    "61.68.41.135:8333",   // dnsseed.bluematt.me
    "62.93.65.111:8333",   // dnsseed.bluematt.me
    "5.161.244.95:8333",   // seed.btc.petertodd.net
    "172.105.64.213:8333", // seed.bitcoin.sipa.be
    "43.154.28.63:8333",   // seed.bitcoin.sipa.be
];

fn our_version() -> VersionMsg {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs(),
    )
    .unwrap_or(i64::MAX);

    let empty_addr = NetworkAddr {
        services: 0,
        ip: [0u8; 16],
        port: 0,
    };

    VersionMsg {
        version: 70015,
        services: services::NODE_NETWORK,
        timestamp: now,
        receiver: empty_addr,
        sender: empty_addr,
        nonce: 0xdead_beef_cafe_1234,
        user_agent: "/mantle:0.1.0/".to_string(),
        start_height: 0,
        relay: false,
    }
}

fn getheaders_payload() -> Vec<u8> {
    // Locator: genesis hash → asks for everything from block 1 onward.
    let genesis = Hash256::from_display_hex(
        "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
    )
    .expect("hardcoded genesis hash must parse");
    let msg = GetHeadersMsg {
        version: 70015,
        locator_hashes: vec![genesis],
        stop_hash: Hash256::ZERO,
    };
    let mut payload = Vec::new();
    msg.encode(&mut payload).expect("Vec<u8> write cannot fail");
    payload
}

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    println!("mantle-node starting");

    for seed in SEED_PEERS {
        let addr: SocketAddrV4 = seed.parse().expect("valid addr");
        println!("\ntrying {addr} ...");

        match try_peer(addr) {
            Ok(true) => return, // got headers — done
            Ok(false) => {
                println!("  no headers received, trying next peer");
            }
            Err(e) => eprintln!("  error: {e}"),
        }
    }

    eprintln!("all seed peers exhausted");
}

fn try_peer(addr: SocketAddrV4) -> Result<bool, mantle::Error> {
    let mut node = Node::new(Magic::Mainnet, 64, our_version())?;
    let peer_id = node.connect(addr)?;
    let inbound = node.inbound_rx.clone();

    let mut headers_requested = false;

    loop {
        // Block until I/O arrives or 20 s elapses — no sleep, no Instant::now.
        if !node.poll_or_timeout(20_000)? {
            println!("  timed out");
            return Ok(false);
        }

        if !headers_requested {
            match node.peer_handshake(peer_id) {
                Some(HandshakeState::Ready) => {
                    println!("  handshake complete — sending getheaders");
                    node.send_direct(peer_id, Command::GetHeaders, &getheaders_payload())?;
                    headers_requested = true;
                }
                None => {
                    println!("  peer disconnected during handshake");
                    return Ok(false);
                }
                _ => {}
            }
        }

        while let Some(msg) = inbound.pop() {
            if handle_message(&msg) {
                return Ok(true);
            }
        }
    }
}

/// Returns true if we received a `headers` message.
fn handle_message(msg: &InboundMsg) -> bool {
    let cmd = Command::from_command_name(mantle::network::CommandName(msg.command));

    println!(
        "  <- {} ({} bytes)",
        cmd.map_or("unknown", |c| c.as_str()),
        msg.payload.len()
    );

    if cmd == Some(Command::Headers) {
        match HeadersMsg::iter_entries::<2_000>(&msg.payload) {
            Ok(iter) => {
                let total = iter.len();
                println!("  received {total} header(s)");
                for (i, result) in HeadersMsg::iter_entries::<2_000>(&msg.payload)
                    .expect("iter_entries succeeded moments ago on same bytes")
                    .enumerate()
                    .take(5)
                {
                    let h = match result {
                        Ok(h) => h,
                        Err(e) => {
                            eprintln!("  failed to decode header: {e}");
                            break;
                        }
                    };
                    let hex = h.hash().to_display_hex();
                    println!(
                        "  [{i}] hash={hex}  version={}  time={}  nonce={}",
                        h.version, h.time, h.nonce
                    );
                }
                if total > 5 {
                    println!("  ... and {} more", total - 5);
                }
                return true;
            }
            Err(e) => eprintln!("  failed to decode headers: {e}"),
        }
    }

    false
}
