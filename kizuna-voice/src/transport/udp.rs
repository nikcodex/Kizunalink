#![allow(dead_code)]
use crate::error::{Error, Result};
use byteorder::{BigEndian, WriteBytesExt};
use std::io::Cursor;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tracing::info;

pub struct VoiceUdp {
    socket: UdpSocket,
    remote_addr: SocketAddr,
}

impl VoiceUdp {
    pub async fn bind_and_discover(
        server_ip: &str,
        server_port: u16,
        ssrc: u32,
    ) -> Result<(Self, String, u16)> {
        let remote_addr: SocketAddr = format!("{}:{}", server_ip, server_port)
            .parse()
            .map_err(|e| Error::Transport(format!("Invalid address: {}", e)))?;

        let socket = UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        socket
            .connect(remote_addr)
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        info!(
            "Bound UDP socket to {}, discovering IP...",
            socket.local_addr().unwrap()
        );

        // IP Discovery packet
        // 74 bytes total.
        // bytes 0-1: Type (1 = request, 2 = response)
        // bytes 2-3: length (70)
        // bytes 4-7: ssrc
        let mut packet = vec![0u8; 74];
        packet[1] = 1; // Type = 1 (request)
        packet[3] = 70; // Length = 70

        let mut cursor = Cursor::new(&mut packet[4..8]);
        cursor.write_u32::<BigEndian>(ssrc).unwrap();

        socket
            .send(&packet)
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        let mut response = vec![0u8; 74];
        let len = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            socket.recv(&mut response),
        )
        .await
        .map_err(|_| Error::Transport("IP discovery request timed out".into()))?
        .map_err(|e| Error::Transport(e.to_string()))?;

        if len < 74 {
            return Err(Error::Transport(
                "Received malformed IP discovery response".into(),
            ));
        }

        // Response string is null terminated starting at byte 8
        let end = response[8..72].iter().position(|&x| x == 0).unwrap_or(64);
        let external_ip = String::from_utf8_lossy(&response[8..8 + end]).into_owned();

        let mut cursor = Cursor::new(&response[72..74]);
        let external_port = byteorder::ReadBytesExt::read_u16::<BigEndian>(&mut cursor).unwrap();

        Ok((
            Self {
                socket,
                remote_addr,
            },
            external_ip,
            external_port,
        ))
    }

    pub async fn send_packet(&self, packet: &[u8]) -> Result<()> {
        self.socket
            .send(packet)
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
        Ok(())
    }
}
