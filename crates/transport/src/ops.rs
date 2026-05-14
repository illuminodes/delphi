pub const OP_CONNECT: u8 = 0;
pub const OP_LINK_TIMEOUT: u8 = 1;
pub const OP_SEND: u8 = 2;
pub const OP_RECV: u8 = 3;
pub const OP_CLOSE: u8 = 4;
pub const OP_TICK: u8 = 0xff;

#[inline]
#[must_use]
pub fn make_user_data(slot: u16, g: u16, op: u8) -> u64 {
    (u64::from(slot) << 16) | (u64::from(g) << 32) | u64::from(op)
}

#[inline]
#[must_use]
pub fn decode_user_data(raw: u64) -> (u16, u8, u16) {
    #[allow(clippy::cast_possible_truncation)]
    let op = (raw & 0xff) as u8;
    #[allow(clippy::cast_possible_truncation)]
    let slot = ((raw >> 16) & 0xffff) as u16;
    #[allow(clippy::cast_possible_truncation)]
    let g = ((raw >> 32) & 0xffff) as u16;
    (slot, op, g)
}

#[inline]
#[must_use]
pub fn make_send_user_data(slot: u16, g: u16, send_seq: u16) -> u64 {
    make_user_data(slot, g, OP_SEND) | (u64::from(send_seq) << 48)
}

#[inline]
#[must_use]
pub fn decode_send_seq(raw: u64) -> u16 {
    #[allow(clippy::cast_possible_truncation)]
    let s = (raw >> 48) as u16;
    s
}

#[inline]
#[must_use]
pub fn decode_cqe_buffer_id(flags: ququmatz::types::CqeFlags) -> Option<u16> {
    use ququmatz::types::CqeFlags;
    if flags.contains(CqeFlags::BUFFER) {
        #[allow(clippy::cast_possible_truncation)]
        Some((flags.bits() >> 16) as u16)
    } else {
        None
    }
}
