use quetzalcoatl::mpsc;
use ququmatz::types::MsgFlags;
use ququmatz::{IoUring, Sqe};

use super::types::{Error, Op, OutboundMsg, UserDataTag};

/// SQ/CQ depth for the TX ring.
const TX_RING_DEPTH: u32 = 256;

/// Drives outbound sends on a dedicated thread.
///
/// Owns its own `IoUring` (TX ring) and the mpsc consumer end of the outbound
/// channel. The hot path: pop an `OutboundMsg` (moves the `Box<[u8]>` — no
/// byte copy), submit a send SQE whose pointer into the boxed slice is kept
/// alive via the `in_flight` vec, reap CQEs to drop completed sends.
pub struct TxThread {
    ring: IoUring,
    outbound: mpsc::Consumer<OutboundMsg>,
}

impl TxThread {
    /// # Errors
    /// Returns [`Error::RingCreate`] if the `io_uring` instance cannot be created.
    pub fn new(outbound: mpsc::Consumer<OutboundMsg>) -> Result<Self, Error> {
        let ring = IoUring::new(TX_RING_DEPTH).map_err(Error::RingCreate)?;
        Ok(Self { ring, outbound })
    }

    /// Block until the outbound channel closes, draining and sending every message.
    ///
    /// # Errors
    /// Returns [`Error::RingSubmit`] or [`Error::RingPush`] on fatal ring failures.
    pub fn run(mut self) -> Result<(), Error> {
        // Boxes whose bytes are referenced by in-flight SQEs. Dropped after
        // the matching send CQE arrives, releasing the allocation.
        let mut in_flight: Vec<(u64, OutboundMsg)> = Vec::with_capacity(TX_RING_DEPTH as usize);

        loop {
            // Reap completed sends first to free in-flight slots.
            self.reap_cqes(&mut in_flight);

            // Pop next message. Block only when the ring is idle to avoid
            // stalling — if sends are in flight we want to keep draining CQEs.
            let msg = if in_flight.is_empty() {
                match self.outbound.pop_block() {
                    Some(m) => m,
                    None => break, // channel closed and drained
                }
            } else if let Some(m) = self.outbound.pop() {
                m
            } else {
                // Nothing ready; wait for at least one CQE then loop.
                self.ring.submit_and_wait(1).map_err(Error::RingSubmit)?;
                continue;
            };

            let tag = UserDataTag::new(msg.peer, Op::Send).raw();
            // SAFETY: Sqe::send stores buf.as_ptr() internally. We keep `msg`
            // in `in_flight` until the matching CQE arrives, so the Box — and
            // the bytes it owns — outlives the kernel operation.
            let sqe = Sqe::send(msg.fd, &msg.bytes, MsgFlags::default()).user_data(tag);

            if let Ok(()) = self.ring.push(sqe) {
                in_flight.push((tag, msg));
                self.ring.submit().map_err(Error::RingSubmit)?;
            } else {
                // SQ full — wait for a CQE slot to free, then retry.
                self.ring.submit_and_wait(1).map_err(Error::RingSubmit)?;
                self.reap_cqes(&mut in_flight);
                let sqe = Sqe::send(msg.fd, &msg.bytes, MsgFlags::default()).user_data(tag);
                self.ring.push(sqe).map_err(Error::RingPush)?;
                in_flight.push((tag, msg));
                self.ring.submit().map_err(Error::RingSubmit)?;
            }
        }

        // Drain remaining CQEs for any still-in-flight sends.
        while !in_flight.is_empty() {
            self.ring.submit_and_wait(1).map_err(Error::RingSubmit)?;
            self.reap_cqes(&mut in_flight);
        }

        Ok(())
    }

    fn reap_cqes(&mut self, in_flight: &mut Vec<(u64, OutboundMsg)>) {
        while let Some(cqe) = self.ring.complete() {
            if let Some(pos) = in_flight.iter().position(|(tag, _)| *tag == cqe.user_data) {
                in_flight.swap_remove(pos); // drops the Box, freeing the bytes
            }
        }
    }
}
