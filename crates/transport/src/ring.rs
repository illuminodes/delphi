use std::io;
use std::time::Duration;

use mantle::network::{Command, Network};
use ququmatz::types::{MsgFlags, RawFd, SockAddrIn, SqeFlags, TimeoutFlags, Timespec};
use ququmatz::{IoUring, ProvidedBufferRing, Sqe};

use crate::arena::SendArena;
use crate::ops::{CqeData, Op, decode_cqe_buffer_id};
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

pub trait IoUringExt {
    fn push_connect(
        &mut self,
        slot: u16,
        g: u16,
        fd: RawFd,
        sockaddr: &SockAddrIn,
        timeout: &Timespec,
    ) -> Result<(), io::Error>;

    #[allow(clippy::too_many_arguments)]
    fn push_send(
        &mut self,
        arena: &mut SendArena,
        network: Network,
        cmd: Command,
        payload: &[u8],
        slot: u16,
        g: u16,
        fd: RawFd,
    ) -> Result<u16, io::Error>;

    fn push_recv_multishot(
        &mut self,
        slot: u16,
        g: u16,
        fd: RawFd,
        buf_group_id: u16,
    ) -> Result<(), io::Error>;

    fn push_tick(&mut self, dur: Duration) -> Result<(), io::Error>;
}

impl IoUringExt for IoUring {
    fn push_connect(
        &mut self,
        slot: u16,
        g: u16,
        fd: RawFd,
        sockaddr: &SockAddrIn,
        timeout: &Timespec,
    ) -> Result<(), io::Error> {
        let sqe = Sqe::connect(fd, &sockaddr.to_bytes())
            .user_data(CqeData::encode(slot, g, Op::Connect))
            .flags(SqeFlags::IO_LINK);
        self.push(sqe).map_err(uring_err)?;
        self.push(
            Sqe::link_timeout(timeout, TimeoutFlags::default()).user_data(CqeData::encode(
                slot,
                g,
                Op::LinkTimeout,
            )),
        )
        .map_err(uring_err)?;
        Ok(())
    }

    fn push_send(
        &mut self,
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
        let handle = arena.handle(send_seq);
        let sqe = Sqe::send(fd, handle.as_slice(), MsgFlags::default())
            .user_data(CqeData::encode_send(slot, g, send_seq));
        let r = self.push(sqe).map_err(uring_err);
        match r {
            Ok(()) => Ok(send_seq),
            Err(e) => {
                arena.release(send_seq);
                Err(e)
            }
        }
    }

    fn push_recv_multishot(
        &mut self,
        slot: u16,
        g: u16,
        fd: RawFd,
        buf_group_id: u16,
    ) -> Result<(), io::Error> {
        self.push(
            Sqe::recv_multishot(fd, MsgFlags::default())
                .buffer_select(buf_group_id)
                .user_data(CqeData::encode(slot, g, Op::Recv)),
        )
        .map_err(uring_err)
    }

    fn push_tick(&mut self, dur: Duration) -> Result<(), io::Error> {
        let ts = Timespec::from_millis(u64::try_from(dur.as_millis()).unwrap_or(u64::MAX));
        self.push(
            Sqe::timeout(&ts, 1, TimeoutFlags::default())
                .user_data(CqeData::encode(0, 0, Op::Tick)),
        )
        .map_err(uring_err)
    }
}

impl<T> Peer<T> {
    pub fn close(&mut self, ring: &mut IoUring, send_arena: &mut SendArena) {
        for seq in self.pending_send_seqs.drain(..) {
            send_arena.release(seq);
        }
        if let Some(sock) = self.sock.take() {
            let fd = sock.fd();
            if let Err(e) = ring.push(
                Sqe::cancel(CqeData::encode(0, self.generation, Op::Recv)).user_data(
                    CqeData::encode(0, self.generation, Op::Close),
                ),
            ) {
                eprintln!("  transport: close cancel-recv push failed for fd={fd:?}: {e:?}");
            }
            if ring
                .push(Sqe::close(fd).user_data(CqeData::encode(0, self.generation, Op::Close)))
                .is_ok()
            {
                // Async close SQE accepted — the kernel owns the fd now.
                // Prevent Socket::drop from closing it synchronously.
                let _ = sock.into_fd();
            } else {
                // Ring full — Socket::drop will close synchronously.
                eprintln!(
                    "  transport: close push failed for fd={fd:?} (closed synchronously)"
                );
            }
        }
    }

    pub fn ingest_recv_cqe(
        &mut self,
        pbr: &mut ProvidedBufferRing,
        send_arena: &mut SendArena,
        cqe_flags: ququmatz::types::CqeFlags,
        res: i32,
        ring: &mut IoUring,
    ) {
        use ququmatz::types::CqeFlags;
        if res <= 0 {
            self.state = PeerState::Dead;
            self.recv_armed = false;
            self.close(ring, send_arena);
            return;
        }
        #[allow(clippy::cast_sign_loss)]
        let n = res as u32;
        if let Some(buf_id) = decode_cqe_buffer_id(cqe_flags) {
            match pbr.buffer(buf_id, n) {
                Some(bytes) => self.recv_buf.extend_from_slice(bytes),
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
            self.recv_armed = false;
        }
    }
}

fn uring_err(e: ququmatz::Error) -> io::Error {
    io::Error::other(format!("{e:?}"))
}
