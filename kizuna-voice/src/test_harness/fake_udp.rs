use byteorder::{BigEndian, WriteBytesExt};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::transport::RtpHeader;

#[derive(Debug, Clone)]
pub struct CapturedRtpPacket {
    pub header: RtpHeader,
    pub payload: Vec<u8>,
    pub raw: Vec<u8>,
    pub received_at: Instant,
}

pub struct FakeVoiceUdpServer {
    _socket: Arc<UdpSocket>,
    port: u16,
    ssrc: u32,
    captured_packets: Arc<Mutex<Vec<CapturedRtpPacket>>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl FakeVoiceUdpServer {
    pub async fn start(ssrc: u32) -> Result<Self, String> {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind fake UDP server: {}", e))?;

        let port = socket
            .local_addr()
            .map_err(|e| format!("Failed to get local addr: {}", e))?
            .port();

        let socket = Arc::new(socket);
        let captured_packets = Arc::new(Mutex::new(Vec::new()));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let socket_clone = socket.clone();
        let captured_clone = captured_packets.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    recv_res = socket_clone.recv_from(&mut buf) => {
                        match recv_res {
                            Ok((len, peer_addr)) => {
                                let data = &buf[..len];
                                // Check if this is an IP Discovery Request (74 bytes, type=1)
                                if len == 74 && data[1] == 1 {
                                    // Send IP Discovery Response (74 bytes, type=2)
                                    let mut response = vec![0u8; 74];
                                    response[1] = 2; // Type = 2 (response)
                                    response[3] = 70; // Length = 70

                                    // Echo SSRC from request
                                    response[4..8].copy_from_slice(&data[4..8]);

                                    // IP string: "127.0.0.1" starting at byte 8
                                    let ip_str = b"127.0.0.1\0";
                                    response[8..8 + ip_str.len()].copy_from_slice(ip_str);

                                    // Port at bytes 72-73 in BigEndian
                                    let peer_port = peer_addr.port();
                                    let mut cursor = Cursor::new(&mut response[72..74]);
                                    cursor.write_u16::<BigEndian>(peer_port).unwrap();

                                    let _ = socket_clone.send_to(&response, peer_addr).await;
                                } else if len >= 12 {
                                    // Parse RTP packet
                                    if let Ok(header) = RtpHeader::read_from(&data[..12]) {
                                        let payload = data[12..].to_vec();
                                        let mut list = captured_clone.lock().await;
                                        list.push(CapturedRtpPacket {
                                            header,
                                            payload,
                                            raw: data.to_vec(),
                                            received_at: Instant::now(),
                                        });
                                    }
                                }
                            }
                            Err(_) => {
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            _socket: socket,
            port,
            ssrc,
            captured_packets,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn ssrc(&self) -> u32 {
        self.ssrc
    }

    pub fn ip(&self) -> &str {
        "127.0.0.1"
    }

    pub async fn get_packets(&self) -> Vec<CapturedRtpPacket> {
        let list = self.captured_packets.lock().await;
        list.clone()
    }

    pub async fn packet_count(&self) -> usize {
        let list = self.captured_packets.lock().await;
        list.len()
    }

    pub async fn clear(&self) {
        let mut list = self.captured_packets.lock().await;
        list.clear();
    }

    pub async fn wait_for_packets(
        &self,
        target_count: usize,
        timeout: Duration,
    ) -> Vec<CapturedRtpPacket> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            let count = self.packet_count().await;
            if count >= target_count {
                return self.get_packets().await;
            }
            sleep(Duration::from_millis(10)).await;
        }
        self.get_packets().await
    }
}

impl Drop for FakeVoiceUdpServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}
