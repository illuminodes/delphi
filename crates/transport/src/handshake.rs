use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mantle::codec::{Decode, Encode};
use mantle::hash::Hash256;
use mantle::network::{
    Command, CommandName, MessageHeader, Network, NetworkAddr, VersionMsg, services,
};
use ququmatz::IoUring;

use crate::arena::SendArena;
use crate::peer::{PEER_SEND_OUTSTANDING_CAP, Peer, PeerState};
use crate::ring::{close_peer, push_send};

#[must_use]
pub fn our_version(user_agent: &str) -> VersionMsg {
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
        version: 70016,
        services: services::NODE_NETWORK,
        timestamp: now,
        receiver: empty,
        sender: empty,
        nonce: 0xfeed_face_dead_beef,
        user_agent: user_agent.to_string(),
        start_height: 0,
        relay: false,
    }
}

pub fn consume_handshake_frames<T>(
    ring: &mut IoUring,
    send_arena: &mut SendArena,
    network: Network,
    slot: u16,
    peer: &mut Peer<T>,
    mut on_frame: impl FnMut(CommandName, &[u8]),
) -> Result<(), io::Error> {
    while peer.recv_buf.len() >= MessageHeader::ENCODED_LEN {
        let Ok(header) = MessageHeader::decode(&mut std::io::Cursor::new(
            &peer.recv_buf[..MessageHeader::ENCODED_LEN],
        )) else {
            break;
        };
        let total = MessageHeader::ENCODED_LEN + header.length() as usize;
        if peer.recv_buf.len() < total {
            break;
        }
        let payload_slice = &peer.recv_buf[MessageHeader::ENCODED_LEN..total];
        if Hash256::checksum(payload_slice) != header.checksum() {
            peer.state = PeerState::Dead;
            close_peer(ring, send_arena, peer);
            return Ok(());
        }
        let command = header.command();
        let cmd = command.as_str();

        let needs_send = matches!(
            (peer.state, cmd),
            (
                PeerState::HandshakeSendVersion | PeerState::HandshakeRecving,
                "version"
            ) | (PeerState::Ready, "ping")
        );
        if needs_send && peer.pending_send_seqs.len() >= PEER_SEND_OUTSTANDING_CAP as usize {
            peer.state = PeerState::Dead;
            close_peer(ring, send_arena, peer);
            return Ok(());
        }

        match (peer.state, cmd) {
            (PeerState::HandshakeSendVersion | PeerState::HandshakeRecving, "version") => {
                let seq = push_send(
                    ring,
                    send_arena,
                    network,
                    Command::Verack,
                    &[],
                    slot,
                    peer.generation,
                    peer.fd.ok_or_else(|| io::Error::other("no fd"))?,
                )?;
                peer.pending_send_seqs.push(seq);
                peer.state = PeerState::HandshakeRecving;
            }
            (PeerState::HandshakeRecving | PeerState::HandshakeSendVersion, "verack") => {
                peer.state = PeerState::Ready;
            }
            (PeerState::Ready, "ping") => {
                let mut nonce = [0u8; 8];
                let copy_len = payload_slice.len().min(8);
                nonce[..copy_len].copy_from_slice(&payload_slice[..copy_len]);
                let seq = push_send(
                    ring,
                    send_arena,
                    network,
                    Command::Pong,
                    &nonce[..copy_len],
                    slot,
                    peer.generation,
                    peer.fd.ok_or_else(|| io::Error::other("no fd"))?,
                )?;
                peer.pending_send_seqs.push(seq);
            }
            (PeerState::Ready, _) => {
                on_frame(header.command(), payload_slice);
            }
            _ => {}
        }
        peer.recv_buf.drain(..total);
    }
    Ok(())
}

pub fn encode_version(user_agent: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    our_version(user_agent)
        .encode(&mut buf)
        .expect("Vec<u8> Write is infallible");
    buf
}
