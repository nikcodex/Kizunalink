use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::Cursor;

pub struct RtpHeader {
    pub version: u8,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

impl RtpHeader {
    pub fn new(sequence: u16, timestamp: u32, ssrc: u32) -> Self {
        Self {
            version: 0x80,      // RTP Version 2
            payload_type: 0x78, // Opus payload type in Discord
            sequence,
            timestamp,
            ssrc,
        }
    }

    pub fn write_to(&self, buf: &mut Vec<u8>) -> std::io::Result<()> {
        buf.write_u8(self.version)?;
        buf.write_u8(self.payload_type)?;
        buf.write_u16::<BigEndian>(self.sequence)?;
        buf.write_u32::<BigEndian>(self.timestamp)?;
        buf.write_u32::<BigEndian>(self.ssrc)?;
        Ok(())
    }

    pub fn read_from(buf: &[u8]) -> std::io::Result<Self> {
        let mut cursor = Cursor::new(buf);
        let version = cursor.read_u8()?;
        let payload_type = cursor.read_u8()?;
        let sequence = cursor.read_u16::<BigEndian>()?;
        let timestamp = cursor.read_u32::<BigEndian>()?;
        let ssrc = cursor.read_u32::<BigEndian>()?;
        Ok(Self {
            version,
            payload_type,
            sequence,
            timestamp,
            ssrc,
        })
    }
}

pub struct RtpPacket {
    pub header: RtpHeader,
    pub payload: Vec<u8>,
}

impl RtpPacket {
    pub fn into_bytes(self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(12 + self.payload.len());
        self.header.write_to(&mut buf).unwrap();
        buf.extend(self.payload);
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtp_header_serialization() {
        let header = RtpHeader::new(1234, 567890, 987654321);
        let mut buf = Vec::new();
        header.write_to(&mut buf).unwrap();

        assert_eq!(buf.len(), 12);

        let decoded = RtpHeader::read_from(&buf).unwrap();
        assert_eq!(decoded.version, 0x80);
        assert_eq!(decoded.payload_type, 0x78);
        assert_eq!(decoded.sequence, 1234);
        assert_eq!(decoded.timestamp, 567890);
        assert_eq!(decoded.ssrc, 987654321);
    }
}
