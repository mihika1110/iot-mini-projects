//! Wire protocol for WhyBlue peer-to-peer communication.
//!
//! All messages between WhyBlue peers use a fixed binary header followed by
//! a variable-length payload. The header supports sequencing, timestamping,
//! traffic classification, and transport hinting for handover-safe delivery.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{HandoverMsg, TrafficClass, WbTransport};

/// Magic number identifying WhyBlue protocol frames: "WB_1"
pub const WB_MAGIC: u32 = 0x57425F31;

/// Current protocol version
pub const WB_VERSION: u16 = 1;

/// Fixed header size in bytes (4+2+2+4+4+8+1+1+2 = 28)
pub const WB_HEADER_SIZE: usize = 28;

// ─── Header Flags ──────────────────────────────────────────────────────────────

pub const FLAG_CONTROL: u16 = 0x0001;
pub const FLAG_PROBE: u16 = 0x0002;
pub const FLAG_HANDOVER: u16 = 0x0004;
pub const FLAG_ACK: u16 = 0x0008;
pub const FLAG_RETRANSMIT: u16 = 0x0010;

// ─── Wire Header ───────────────────────────────────────────────────────────────

/// 28-byte binary header for all WhyBlue frames.
#[derive(Debug, Clone)]
pub struct WbHeader {
    /// Magic number (must be WB_MAGIC)
    pub magic: u32,
    /// Protocol version
    pub version: u16,
    /// Bitfield flags (FLAG_*)
    pub flags: u16,
    /// Session identifier (agreed during Hello exchange)
    pub session_id: u32,
    /// Monotonic sequence number per session
    pub seq_no: u32,
    /// Timestamp in nanoseconds since UNIX epoch
    pub timestamp_ns: u64,
    /// Traffic class of the payload
    pub traffic_class: u8,
    /// Hint about which transport to use for reply
    pub transport_hint: u8,
    /// Length of the payload following this header
    pub payload_len: u16,
}

impl WbHeader {
    /// Create a new header with common defaults filled in.
    pub fn new(
        session_id: u32,
        seq_no: u32,
        traffic_class: TrafficClass,
        transport_hint: WbTransport,
        payload_len: u16,
        flags: u16,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            magic: WB_MAGIC,
            version: WB_VERSION,
            flags,
            session_id,
            seq_no,
            timestamp_ns: now.as_nanos() as u64,
            traffic_class: traffic_class_to_u8(traffic_class),
            transport_hint: transport_to_u8(transport_hint),
            payload_len,
        }
    }

    /// Encode the header into exactly 28 bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(WB_HEADER_SIZE);
        buf.write_u32::<BigEndian>(self.magic).unwrap();
        buf.write_u16::<BigEndian>(self.version).unwrap();
        buf.write_u16::<BigEndian>(self.flags).unwrap();
        buf.write_u32::<BigEndian>(self.session_id).unwrap();
        buf.write_u32::<BigEndian>(self.seq_no).unwrap();
        buf.write_u64::<BigEndian>(self.timestamp_ns).unwrap();
        buf.write_u8(self.traffic_class).unwrap();
        buf.write_u8(self.transport_hint).unwrap();
        buf.write_u16::<BigEndian>(self.payload_len).unwrap();
        buf
    }

    /// Decode a header from a byte slice (must be at least WB_HEADER_SIZE bytes).
    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        if data.len() < WB_HEADER_SIZE {
            return Err(ProtocolError::TooShort {
                got: data.len(),
                need: WB_HEADER_SIZE,
            });
        }
        let mut cursor = Cursor::new(data);
        let magic = cursor.read_u32::<BigEndian>()?;
        if magic != WB_MAGIC {
            return Err(ProtocolError::BadMagic(magic));
        }
        let version = cursor.read_u16::<BigEndian>()?;
        let flags = cursor.read_u16::<BigEndian>()?;
        let session_id = cursor.read_u32::<BigEndian>()?;
        let seq_no = cursor.read_u32::<BigEndian>()?;
        let timestamp_ns = cursor.read_u64::<BigEndian>()?;
        let traffic_class = cursor.read_u8()?;
        let transport_hint = cursor.read_u8()?;
        let payload_len = cursor.read_u16::<BigEndian>()?;

        Ok(Self {
            magic,
            version,
            flags,
            session_id,
            seq_no,
            timestamp_ns,
            traffic_class,
            transport_hint,
            payload_len,
        })
    }
}

// ─── Full Frame Encoding/Decoding ──────────────────────────────────────────────

/// Encode a complete WhyBlue frame (header + payload).
pub fn encode_frame(header: &WbHeader, payload: &[u8]) -> Vec<u8> {
    let mut frame = header.encode();
    frame.extend_from_slice(payload);
    frame
}

/// Decode a complete frame, returning the header and payload slice.
pub fn decode_frame(data: &[u8]) -> Result<(WbHeader, &[u8]), ProtocolError> {
    let header = WbHeader::decode(data)?;
    let payload_start = WB_HEADER_SIZE;
    let payload_end = payload_start + header.payload_len as usize;
    if data.len() < payload_end {
        return Err(ProtocolError::TooShort {
            got: data.len(),
            need: payload_end,
        });
    }
    Ok((header, &data[payload_start..payload_end]))
}

// ─── Probe Messages ────────────────────────────────────────────────────────────

/// Probe request: sent periodically on each transport to measure RTT and liveness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingProbe {
    pub seq: u32,
    pub send_ts_ns: u64,
}

/// Probe response: echoes back the original timestamp for RTT calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PongProbe {
    pub seq: u32,
    pub send_ts_ns: u64,
    pub echo_ts_ns: u64,
}

impl PingProbe {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(12);
        buf.write_u32::<BigEndian>(self.seq).unwrap();
        buf.write_u64::<BigEndian>(self.send_ts_ns).unwrap();
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        if data.len() < 12 {
            return Err(ProtocolError::TooShort {
                got: data.len(),
                need: 12,
            });
        }
        let mut cursor = Cursor::new(data);
        Ok(Self {
            seq: cursor.read_u32::<BigEndian>()?,
            send_ts_ns: cursor.read_u64::<BigEndian>()?,
        })
    }
}

impl PongProbe {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(20);
        buf.write_u32::<BigEndian>(self.seq).unwrap();
        buf.write_u64::<BigEndian>(self.send_ts_ns).unwrap();
        buf.write_u64::<BigEndian>(self.echo_ts_ns).unwrap();
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        if data.len() < 20 {
            return Err(ProtocolError::TooShort {
                got: data.len(),
                need: 20,
            });
        }
        let mut cursor = Cursor::new(data);
        Ok(Self {
            seq: cursor.read_u32::<BigEndian>()?,
            send_ts_ns: cursor.read_u64::<BigEndian>()?,
            echo_ts_ns: cursor.read_u64::<BigEndian>()?,
        })
    }
}

// ─── Handover Message Encoding ─────────────────────────────────────────────────

impl HandoverMsg {
    /// Encode a handover control message to bytes.
    pub fn encode(&self) -> Vec<u8> {
        // Use simple JSON for handover messages (they're small and infrequent)
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Decode a handover control message from bytes.
    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        serde_json::from_slice(data).map_err(|e| ProtocolError::InvalidPayload(e.to_string()))
    }
}

// ─── Converters ────────────────────────────────────────────────────────────────

fn traffic_class_to_u8(tc: TrafficClass) -> u8 {
    match tc {
        TrafficClass::Control => 0,
        TrafficClass::Interactive => 1,
        TrafficClass::Stream => 2,
        TrafficClass::Bulk => 3,
    }
}

pub fn u8_to_traffic_class(v: u8) -> TrafficClass {
    match v {
        0 => TrafficClass::Control,
        1 => TrafficClass::Interactive,
        2 => TrafficClass::Stream,
        _ => TrafficClass::Bulk,
    }
}

fn transport_to_u8(t: WbTransport) -> u8 {
    match t {
        WbTransport::None => 0,
        WbTransport::Bluetooth => 1,
        WbTransport::Wifi => 2,
    }
}

pub fn u8_to_transport(v: u8) -> WbTransport {
    match v {
        1 => WbTransport::Bluetooth,
        2 => WbTransport::Wifi,
        _ => WbTransport::None,
    }
}

// ─── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame too short: got {got} bytes, need {need}")]
    TooShort { got: usize, need: usize },
    #[error("bad magic number: 0x{0:08X}")]
    BadMagic(u32),
    #[error("invalid payload: {0}")]
    InvalidPayload(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_roundtrip() {
        let header = WbHeader::new(42, 100, TrafficClass::Interactive, WbTransport::Wifi, 256, FLAG_CONTROL);
        let encoded = header.encode();
        assert_eq!(encoded.len(), WB_HEADER_SIZE);
        let decoded = WbHeader::decode(&encoded).unwrap();
        assert_eq!(decoded.magic, WB_MAGIC);
        assert_eq!(decoded.session_id, 42);
        assert_eq!(decoded.seq_no, 100);
        assert_eq!(decoded.payload_len, 256);
    }

    #[test]
    fn test_frame_roundtrip() {
        let header = WbHeader::new(1, 1, TrafficClass::Control, WbTransport::Bluetooth, 5, 0);
        let payload = b"hello";
        let frame = encode_frame(&header, payload);
        let (dec_h, dec_p) = decode_frame(&frame).unwrap();
        assert_eq!(dec_h.session_id, 1);
        assert_eq!(dec_p, b"hello");
    }

    #[test]
    fn test_probe_roundtrip() {
        let ping = PingProbe {
            seq: 42,
            send_ts_ns: 123456789,
        };
        let encoded = ping.encode();
        let decoded = PingProbe::decode(&encoded).unwrap();
        assert_eq!(decoded.seq, 42);
        assert_eq!(decoded.send_ts_ns, 123456789);
    }

    #[test]
    fn test_bad_magic_rejected() {
        let mut data = vec![0u8; WB_HEADER_SIZE];
        data[0..4].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(WbHeader::decode(&data).is_err());
    }

    #[test]
    fn test_handover_msg_roundtrip() {
        let msg = HandoverMsg::SwitchPrepare {
            generation: 5,
            new_transport: WbTransport::Wifi,
        };
        let encoded = msg.encode();
        let decoded = HandoverMsg::decode(&encoded).unwrap();
        match decoded {
            HandoverMsg::SwitchPrepare { generation, new_transport } => {
                assert_eq!(generation, 5);
                assert_eq!(new_transport, WbTransport::Wifi);
            }
            _ => panic!("wrong variant"),
        }
    }
}
