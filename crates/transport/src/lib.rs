#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, clippy::must_use_candidate)]

mod arena;
mod handshake;
mod ops;
mod peer;
mod ring;

pub use arena::SendArena;
pub use handshake::{consume_handshake_frames, encode_version, our_version};
pub use ops::{
    OP_CLOSE, OP_CONNECT, OP_LINK_TIMEOUT, OP_RECV, OP_SEND, OP_TICK, decode_cqe_buffer_id,
    decode_send_seq, decode_user_data, make_send_user_data, make_user_data,
};
pub use peer::{PEER_SEND_OUTSTANDING_CAP, Peer, PeerPool, PeerState};
pub use ring::{
    close_peer, ingest_recv_cqe, make_ring, push_connect, push_recv_multishot, push_send,
    push_tick,
};

pub use ququmatz;
