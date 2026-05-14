use std::io;
use std::time::Duration;

use mantle::network::{Command, Network};
use ququmatz::types::{MsgFlags, RawFd, SockAddrIn, SqeFlags, TimeoutFlags, Timespec};
use ququmatz::{IoUring, ProvidedBufferRing, Sqe};

use crate::arena::SendArena;
use crate::ops::{
    OP_CLOSE, OP_CONNECT, OP_LINK_TIMEOUT, OP_RECV, OP_TICK, decode_cqe_buffer_id,
    make_send_user_data, make_user_data,
};
use crate::peer::{Peer, PeerState};

const RING_ENTRIES: u32 = 32_768;

pub fn make_ring() -> Result<IoUring, io::Error> {
    let bare = std::env::var_os("MANTLE_URING_BARE").is_some_and(|v| v == "1");
    if bare {
        eprintln!("  io_uring: bare (MANTLE_URING_BARE=1)");
    } else if let Ok(ring) = IoUring::builder(RING_ENTRIES).single_issuer().build() {
        return Ok(ring);
    }
    IoUring::new(RING_ENTRIES).map_err(|e| io::Error::other(format!("IoUring::new: {e:?}")))
}

pub fn push_connect(
    ring: &mut IoUring,
    slot: u16,
    g: u16,
    fd: RawFd,
    sockaddr: &SockAddrIn,
    timeout: &Timespec,
) -> Result<(), io::Error> {
    let addr_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref::<SockAddrIn>(sockaddr).cast(),
            std::mem::size_of::<SockAddrIn>(),
        )
    };
    ring.push(
        Sqe::connect(fd, addr_bytes)
            .user_data(make_user_data(slot, g, OP_CONNECT))
            .flags(SqeFlags::IO_LINK),
    )
    .map_err(uring_err)?;
    ring.push(
        Sqe::link_timeout(timeout, TimeoutFlags::default()).user_data(make_user_data(
            slot,
            g,
            OP_LINK_TIMEOUT,
        )),
    )
    .map_err(uring_err)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn push_send(
    ring: &mut IoUring,
    arena: &mut SendArena,
    network: Network,
    cmd: Command,
    payload: &[u8],
    slot: u16,
    g: u16,
    fd: RawFd,
) -> Result<u16, io::Error> {
    let send_seq = arena
        .frame_into(network.magic(), cmd, payload)
        .ok_or_else(|| {
            io::Error::other(format!(
                "send arena exhausted (peer slot={slot} g={g}, free={} cap={})",
                arena.free_count(),
                SendArena::slot_count()
            ))
        })?;
    let bytes = arena.slice(send_seq);
    let bytes_static: &[u8] = unsafe { std::slice::from_raw_parts(bytes.as_ptr(), bytes.len()) };
    let r = ring
        .push(
            Sqe::send(fd, bytes_static, MsgFlags::default())
                .user_data(make_send_user_data(slot, g, send_seq)),
        )
        .map_err(uring_err);
    match r {
        Ok(()) => Ok(send_seq),
        Err(e) => {
            arena.release(send_seq);
            Err(e)
        }
    }
}

pub fn push_recv_multishot(
    ring: &mut IoUring,
    slot: u16,
    g: u16,
    fd: RawFd,
    buf_group_id: u16,
) -> Result<(), io::Error> {
    ring.push(
        Sqe::recv_multishot(fd, MsgFlags::default())
            .buffer_select(buf_group_id)
            .user_data(make_user_data(slot, g, OP_RECV)),
    )
    .map_err(uring_err)
}

pub fn push_tick(ring: &mut IoUring, dur: Duration) -> Result<(), io::Error> {
    let ts = Timespec::from_millis(u64::try_from(dur.as_millis()).unwrap_or(u64::MAX));
    ring.push(
        Sqe::timeout(&ts, 1, TimeoutFlags::default()).user_data(make_user_data(0, 0, OP_TICK)),
    )
    .map_err(uring_err)
}

pub fn close_peer<T>(ring: &mut IoUring, send_arena: &mut SendArena, peer: &mut Peer<T>) {
    for seq in peer.pending_send_seqs.drain(..) {
        send_arena.release(seq);
    }
    if let Some(fd) = peer.fd.take() {
        if let Err(e) = ring.push(
            Sqe::cancel(make_user_data(0, peer.generation, OP_RECV)).user_data(make_user_data(
                0,
                peer.generation,
                OP_CLOSE,
            )),
        ) {
            eprintln!("  transport: close_peer cancel-recv push failed for fd={fd:?}: {e:?}");
        }
        if let Err(e) =
            ring.push(Sqe::close(fd).user_data(make_user_data(0, peer.generation, OP_CLOSE)))
        {
            drop(unsafe { ququmatz::Socket::from_fd(fd) });
            eprintln!(
                "  transport: close_peer close push failed for fd={fd:?}: {e:?} \
                 (closed synchronously)"
            );
        }
    }
}

pub fn ingest_recv_cqe<T>(
    pbr: &mut ProvidedBufferRing,
    send_arena: &mut SendArena,
    peer: &mut Peer<T>,
    cqe_flags: ququmatz::types::CqeFlags,
    res: i32,
    ring: &mut IoUring,
) {
    use ququmatz::types::CqeFlags;
    if res <= 0 {
        peer.state = PeerState::Dead;
        peer.recv_armed = false;
        close_peer(ring, send_arena, peer);
        return;
    }
    #[allow(clippy::cast_sign_loss)]
    let n = res as u32;
    if let Some(buf_id) = decode_cqe_buffer_id(cqe_flags) {
        match pbr.buffer(buf_id, n) {
            Some(bytes) => peer.recv_buf.extend_from_slice(bytes),
            None => {
                eprintln!(
                    "  transport: pbr.buffer({buf_id}, {n}) returned None \
                     (buf_size={}, entries={})",
                    pbr.buf_size(),
                    pbr.entries(),
                );
            }
        }
        pbr.recycle(buf_id);
    } else {
        eprintln!(
            "  transport: recv CQE with res={n} has no BUFFER flag \
             (multishot setup is broken)"
        );
    }
    if !cqe_flags.contains(CqeFlags::MORE) {
        peer.recv_armed = false;
    }
}

fn uring_err(e: ququmatz::Error) -> io::Error {
    io::Error::other(format!("{e:?}"))
}
