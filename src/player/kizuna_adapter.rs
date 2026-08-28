use std::sync::Arc;
use tokio::sync::Mutex;
use kizuna_voice::connection::session::VoiceSession;
use kizuna_voice::gateway::VoiceGatewayClient;
use kizuna_voice::transport::VoiceUdp;
use tracing::{info, warn, error};

pub struct KizunaVoiceAdapter {
    session: VoiceSession,
    gateway: Option<VoiceGatewayClient>,
    udp: Option<VoiceUdp>,
}

impl KizunaVoiceAdapter {
    pub fn new(session_id: String, token: String, endpoint: String) -> Self {
        Self {
            session: VoiceSession::new(session_id, token, endpoint),
            gateway: None,
            udp: None,
        }
    }

    pub async fn connect(&mut self, server_id: &str, user_id: &str) -> Result<(), String> {
        info!("Connecting using KizunaVoice adapter...");
        
        let mut gw = VoiceGatewayClient::connect(&self.session.endpoint)
            .await.map_err(|e| e.to_string())?;
            
        gw.send_identify(server_id, user_id, &self.session.session_id, &self.session.token)
            .await.map_err(|e| e.to_string())?;
            
        self.gateway = Some(gw);
        Ok(())
    }
    
    // Stub for sending audio frames via the new engine
    pub async fn send_audio(&self, _pcm_or_opus: &[u8]) {
        // To be implemented using kizuna-voice audio abstraction
    }
}
