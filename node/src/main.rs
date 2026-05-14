#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use std::net::SocketAddrV4;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mantle::network::{
    Command, GetHeadersMsg, HeadersMsg, Network, NetworkAddr, VersionMsg, services,
};
use mantle::peer::{InboundMsg, Node, OutboundMsg};

const NETWORK: Network = Network::Mainnet;

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
    GetHeadersMsg::single_locator(NETWORK.genesis_hash()).encode_to_vec()
}

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    println!("mantle-node starting");

    for seed in NETWORK.default_seeds() {
        let addr: SocketAddrV4 = seed.parse().expect("valid addr");
        println!("\ntrying {addr} ...");

        match try_peer(addr) {
            Ok(true) => return,
            Ok(false) => println!("  no headers received, trying next peer"),
            Err(e) => eprintln!("  error: {e}"),
        }
    }

    eprintln!("all seed peers exhausted");
}

fn try_peer(addr: SocketAddrV4) -> Result<bool, mantle::peer::Error> {
    let node = Node::new(NETWORK.magic(), our_version())?;
    let (peer_id, fd) = node.connect(addr)?;
    let inbound = node.inbound_rx.clone();

    let mut headers_requested = false;

    loop {
        let Some(msg) = inbound.pop_block() else {
            return Ok(false);
        };

        match msg {
            InboundMsg::Tick => {}

            InboundMsg::Network {
                peer,
                command,
                payload,
            } => {
                if peer != peer_id {
                    continue;
                }
                let cmd = Command::from_command_name(mantle::network::CommandName(command));

                if !headers_requested {
                    if cmd == Some(Command::Version) || cmd == Some(Command::Verack) {
                        // Still in handshake — wait for first non-handshake message.
                        continue;
                    }
                    // First application message means handshake is done.
                    println!("  handshake complete — sending getheaders");
                    let raw = getheaders_payload();
                    let mut framed = Vec::new();
                    NETWORK
                        .magic()
                        .frame(Command::GetHeaders, &raw, &mut framed)
                        .expect("frame cannot fail");
                    let _ = node.outbound_tx.push(OutboundMsg {
                        peer: peer_id,
                        fd,
                        bytes: framed.into_boxed_slice(),
                    });
                    headers_requested = true;
                }

                println!(
                    "  <- {} ({} bytes)",
                    cmd.map_or("unknown", |c| c.as_str()),
                    payload.len()
                );

                if cmd == Some(Command::Headers) {
                    match HeadersMsg::iter_entries::<2_000>(&payload) {
                        Ok(iter) => {
                            let total = iter.len();
                            println!("  received {total} header(s)");
                            for (i, result) in HeadersMsg::iter_entries::<2_000>(&payload)
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
                            return Ok(true);
                        }
                        Err(e) => eprintln!("  failed to decode headers: {e}"),
                    }
                }
            }
        }
    }
}
