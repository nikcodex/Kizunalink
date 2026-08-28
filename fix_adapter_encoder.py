import re

with open('src/player/kizuna_adapter.rs', 'r') as f:
    content = f.read()

content = content.replace(
'''        tokio::spawn(async move {
            let mut encoder = OpusEncoder::new().unwrap();
            
            scheduler.run(cmd_rx, event_tx, |frame| {
                let udp = udp.clone();
                let dave = dave.clone();
                let sender_id_clone = sender_id.clone();
                
                async move {
                    sequence = sequence.wrapping_add(1);
                    timestamp = timestamp.wrapping_add(960);
                    
                    let opus_data = match frame {
                        AudioFrame::Opus(data) => data,
                        AudioFrame::Pcm(pcm) => {
                            let encoded = encoder.encode(OpusSource::Pcm(pcm)).unwrap();
                            if let AudioFrame::Opus(data) = encoded { data } else { vec![] }
                        }
                    };''',
'''        tokio::spawn(async move {
            let encoder = std::sync::Arc::new(tokio::sync::Mutex::new(OpusEncoder::new().unwrap()));
            
            scheduler.run(cmd_rx, event_tx, |frame| {
                let udp = udp.clone();
                let dave = dave.clone();
                let sender_id_clone = sender_id.clone();
                let enc_clone = encoder.clone();
                
                async move {
                    sequence = sequence.wrapping_add(1);
                    timestamp = timestamp.wrapping_add(960);
                    
                    let opus_data = match frame {
                        AudioFrame::Opus(data) => data,
                        AudioFrame::Pcm(pcm) => {
                            let mut enc = enc_clone.lock().await;
                            let encoded = enc.encode(OpusSource::Pcm(pcm)).unwrap();
                            if let AudioFrame::Opus(data) = encoded { data } else { vec![] }
                        }
                    };'''
)

with open('src/player/kizuna_adapter.rs', 'w') as f:
    f.write(content)
