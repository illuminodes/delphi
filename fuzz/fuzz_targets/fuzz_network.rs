#![no_main]
use libfuzzer_sys::fuzz_target;
use mantle_core::codec::{Decode, Encode};
use mantle_core::network::{
    GetHeadersMsg, HeadersMsg, InvMsg, MessageHeader, NetworkAddr, PingMsg, VersionMsg,
};
use std::io::Cursor;

macro_rules! fuzz_roundtrip {
    ($T:ty, $data:expr) => {
        if let Ok(v) = <$T>::decode(&mut Cursor::new($data)) {
            let mut buf = Vec::new();
            v.encode(&mut buf).unwrap();
            let v2 = <$T>::decode(&mut Cursor::new(buf.as_slice())).unwrap();
            assert_eq!(v, v2, "round-trip mismatch for {}", stringify!($T));
        }
    };
}

fuzz_target!(|data: &[u8]| {
    // MessageHeader doesn't derive PartialEq; check field-by-field.
    if let Ok(h) = MessageHeader::decode(&mut Cursor::new(data)) {
        let mut buf = Vec::new();
        h.encode(&mut buf).unwrap();
        let h2 = MessageHeader::decode(&mut Cursor::new(buf.as_slice())).unwrap();
        assert_eq!(h.magic, h2.magic);
        assert_eq!(h.command, h2.command);
        assert_eq!(h.length, h2.length);
        assert_eq!(h.checksum, h2.checksum);
    }
    fuzz_roundtrip!(NetworkAddr, data);
    fuzz_roundtrip!(PingMsg, data);
    fuzz_roundtrip!(VersionMsg, data);
    fuzz_roundtrip!(InvMsg, data);
    fuzz_roundtrip!(GetHeadersMsg, data);
    fuzz_roundtrip!(HeadersMsg, data);
});
