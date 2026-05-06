use std::io::{self, Read, Write};

pub trait Encode {
    fn encode(&self, w: &mut impl Write) -> io::Result<()>;
}

pub trait Decode: Sized {
    fn decode(r: &mut impl Read) -> io::Result<Self>;
}

/// Zero-copy decoding from a borrowed byte slice.
/// Returns the decoded value and the remaining unconsumed bytes.
pub trait DecodeRef<'a>: Sized {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])>;
}

// ── Primitives ────────────────────────────────────────────────────────────────

macro_rules! impl_codec_int {
    ($t:ty) => {
        impl Encode for $t {
            #[inline]
            fn encode(&self, w: &mut impl Write) -> io::Result<()> {
                w.write_all(&self.to_le_bytes())
            }
        }

        impl Decode for $t {
            #[inline]
            fn decode(r: &mut impl Read) -> io::Result<Self> {
                let mut buf = [0u8; size_of::<$t>()];
                r.read_exact(&mut buf)?;
                Ok(<$t>::from_le_bytes(buf))
            }
        }

        impl<'a> DecodeRef<'a> for $t {
            #[inline]
            fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
                const N: usize = size_of::<$t>();
                if buf.len() < N {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "not enough bytes",
                    ));
                }
                let (head, tail) = buf.split_at(N);
                Ok((<$t>::from_le_bytes(head.try_into().unwrap()), tail))
            }
        }
    };
}

impl_codec_int!(u8);
impl_codec_int!(u16);
impl_codec_int!(u32);
impl_codec_int!(u64);
impl_codec_int!(i32);
impl_codec_int!(i64);

// ── VarInt ────────────────────────────────────────────────────────────────────

/// Bitcoin variable-length integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VarInt(pub u64);

impl VarInt {
    #[inline]
    pub fn encoded_len(self) -> usize {
        match self.0 {
            0..=0xfc => 1,
            0xfd..=0xffff => 3,
            0x10000..=0xffff_ffff => 5,
            _ => 9,
        }
    }
}

impl Encode for VarInt {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        match self.0 {
            n @ 0..=0xfc => w.write_all(&[n as u8]),
            n @ 0xfd..=0xffff => {
                w.write_all(&[0xfd])?;
                w.write_all(&(n as u16).to_le_bytes())
            }
            n @ 0x10000..=0xffff_ffff => {
                w.write_all(&[0xfe])?;
                w.write_all(&(n as u32).to_le_bytes())
            }
            n => {
                w.write_all(&[0xff])?;
                w.write_all(&n.to_le_bytes())
            }
        }
    }
}

impl Decode for VarInt {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let mut tag = [0u8; 1];
        r.read_exact(&mut tag)?;
        let value = match tag[0] {
            0xff => {
                let mut buf = [0u8; 8];
                r.read_exact(&mut buf)?;
                u64::from_le_bytes(buf)
            }
            0xfe => {
                let mut buf = [0u8; 4];
                r.read_exact(&mut buf)?;
                u32::from_le_bytes(buf) as u64
            }
            0xfd => {
                let mut buf = [0u8; 2];
                r.read_exact(&mut buf)?;
                u16::from_le_bytes(buf) as u64
            }
            n => n as u64,
        };
        Ok(VarInt(value))
    }
}

impl<'a> DecodeRef<'a> for VarInt {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (&tag, rest) = buf
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"))?;
        let (value, tail) = match tag {
            0xff => {
                let (v, tail) = u64::decode_ref(rest)?;
                (v, tail)
            }
            0xfe => {
                let (v, tail) = u32::decode_ref(rest)?;
                (v as u64, tail)
            }
            0xfd => {
                let (v, tail) = u16::decode_ref(rest)?;
                (v as u64, tail)
            }
            n => (n as u64, rest),
        };
        Ok((VarInt(value), tail))
    }
}

// ── Fixed-size byte arrays ────────────────────────────────────────────────────

impl<const N: usize> Encode for [u8; N] {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(self)
    }
}

impl<const N: usize> Decode for [u8; N] {
    #[inline]
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let mut buf = [0u8; N];
        r.read_exact(&mut buf)?;
        Ok(buf)
    }
}

impl<'a, const N: usize> DecodeRef<'a> for [u8; N] {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        if buf.len() < N {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "not enough bytes",
            ));
        }
        let (head, tail) = buf.split_at(N);
        Ok((head.try_into().unwrap(), tail))
    }
}

// ── Borrowed byte slice (zero-copy) ──────────────────────────────────────────

/// Zero-copy view into a VarInt-prefixed byte field.
impl<'a> DecodeRef<'a> for &'a [u8] {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (VarInt(len), rest) = VarInt::decode_ref(buf)?;
        let len = len as usize;
        if rest.len() < len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "not enough bytes",
            ));
        }
        let (data, tail) = rest.split_at(len);
        Ok((data, tail))
    }
}

/// Zero-copy view into a VarInt-prefixed UTF-8 string field.
impl<'a> DecodeRef<'a> for &'a str {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (bytes, tail) = <&'a [u8]>::decode_ref(buf)?;
        let s = std::str::from_utf8(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok((s, tail))
    }
}

// ── Owned VarStr / VarBytes (stream path) ────────────────────────────────────

impl Encode for str {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        VarInt(self.len() as u64).encode(w)?;
        w.write_all(self.as_bytes())
    }
}

impl Encode for String {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.as_str().encode(w)
    }
}

impl Decode for String {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let len = VarInt::decode(r)?.0 as usize;
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf)?;
        String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

impl Encode for Vec<u8> {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        VarInt(self.len() as u64).encode(w)?;
        w.write_all(self)
    }
}

impl Decode for Vec<u8> {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let len = VarInt::decode(r)?.0 as usize;
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf)?;
        Ok(buf)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn roundtrip_encode<T: Encode>(val: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        val.encode(&mut buf).unwrap();
        buf
    }

    fn roundtrip_decode<T: Decode>(bytes: &[u8]) -> T {
        T::decode(&mut Cursor::new(bytes)).unwrap()
    }

    fn decode_ref<'a, T: DecodeRef<'a>>(buf: &'a [u8]) -> (T, &'a [u8]) {
        T::decode_ref(buf).unwrap()
    }

    #[test]
    fn u32_le() {
        let bytes = roundtrip_encode(&0x01020304u32);
        assert_eq!(bytes, [0x04, 0x03, 0x02, 0x01]);
        assert_eq!(roundtrip_decode::<u32>(&bytes), 0x01020304u32);
    }

    #[test]
    fn u32_decode_ref() {
        let buf = [0x04u8, 0x03, 0x02, 0x01, 0xff];
        let (val, tail) = decode_ref::<u32>(&buf);
        assert_eq!(val, 0x01020304u32);
        assert_eq!(tail, &[0xff]);
    }

    #[test]
    fn varint_1byte() {
        let bytes = roundtrip_encode(&VarInt(0xfc));
        assert_eq!(bytes, [0xfc]);
        assert_eq!(roundtrip_decode::<VarInt>(&bytes), VarInt(0xfc));
    }

    #[test]
    fn varint_3byte() {
        let bytes = roundtrip_encode(&VarInt(0xfd));
        assert_eq!(bytes, [0xfd, 0xfd, 0x00]);
        assert_eq!(roundtrip_decode::<VarInt>(&bytes), VarInt(0xfd));
    }

    #[test]
    fn varint_5byte() {
        let bytes = roundtrip_encode(&VarInt(0x10000));
        assert_eq!(bytes, [0xfe, 0x00, 0x00, 0x01, 0x00]);
        assert_eq!(roundtrip_decode::<VarInt>(&bytes), VarInt(0x10000));
    }

    #[test]
    fn varint_9byte() {
        let val = VarInt(u64::MAX);
        let bytes = roundtrip_encode(&val);
        assert_eq!(bytes[0], 0xff);
        assert_eq!(roundtrip_decode::<VarInt>(&bytes), val);
    }

    #[test]
    fn varint_decode_ref() {
        let buf = [0xfd, 0x00, 0x01, 0xab];
        let (val, tail) = decode_ref::<VarInt>(&buf);
        assert_eq!(val, VarInt(0x0100));
        assert_eq!(tail, &[0xab]);
    }

    #[test]
    fn fixed_array() {
        let arr: [u8; 4] = [1, 2, 3, 4];
        let bytes = roundtrip_encode(&arr);
        assert_eq!(bytes, [1, 2, 3, 4]);
        assert_eq!(roundtrip_decode::<[u8; 4]>(&bytes), arr);
    }

    #[test]
    fn fixed_array_decode_ref() {
        let buf = [1u8, 2, 3, 4, 5, 6];
        let (arr, tail) = decode_ref::<[u8; 4]>(&buf);
        assert_eq!(arr, [1, 2, 3, 4]);
        assert_eq!(tail, &[5, 6]);
    }

    #[test]
    fn varstr() {
        let s = "hello".to_string();
        let bytes = roundtrip_encode(&s);
        assert_eq!(bytes[0], 5);
        assert_eq!(&bytes[1..], b"hello");
        assert_eq!(roundtrip_decode::<String>(&bytes), s);
    }

    #[test]
    fn str_decode_ref_zero_copy() {
        let buf = b"\x05helloworld";
        let (s, tail) = decode_ref::<&str>(buf);
        assert_eq!(s, "hello");
        assert_eq!(tail, b"world");
        // s borrows directly from buf — no allocation
        assert!(std::ptr::eq(s.as_bytes().as_ptr(), &buf[1]));
    }

    #[test]
    fn slice_decode_ref_zero_copy() {
        let buf = b"\x04\xde\xad\xbe\xefXX";
        let (data, tail) = decode_ref::<&[u8]>(buf);
        assert_eq!(data, &[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(tail, b"XX");
        assert!(std::ptr::eq(data.as_ptr(), &buf[1]));
    }

    #[test]
    fn vec_u8() {
        let v: Vec<u8> = vec![0xde, 0xad, 0xbe, 0xef];
        let bytes = roundtrip_encode(&v);
        assert_eq!(bytes[0], 4);
        assert_eq!(roundtrip_decode::<Vec<u8>>(&bytes), v);
    }
}
