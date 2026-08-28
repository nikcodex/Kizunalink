import re

with open('kizuna-voice/src/dave/protocol.rs', 'r') as f:
    content = f.read()

content = content.replace(
    '''    pub fn encrypt_frame(
        &mut self,
        sender_id: &str,
        plaintext: &[u8],
        nonce_counter: u32,
    ) -> Result<Vec<u8>, String> {''',
    '''    pub fn encrypt_frame(
        &mut self,
        sender_id: &str,
        plaintext: &[u8],
        nonce_counter: u32,
        rtp_header: &[u8],
    ) -> Result<Vec<u8>, String> {'''
)

content = content.replace(
    '''        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| format!("Encryption failed: {}", e))?;''',
    '''        let payload = aes_gcm::aead::Payload {
            msg: plaintext,
            aad: rtp_header,
        };
        let ciphertext = cipher
            .encrypt(nonce, payload)
            .map_err(|e| format!("Encryption failed: {}", e))?;'''
)

with open('kizuna-voice/src/dave/protocol.rs', 'w') as f:
    f.write(content)

with open('src/player/kizuna_adapter.rs', 'r') as f:
    adapter = f.read()

adapter = adapter.replace(
    'dave_guard.encrypt_frame(&sender_id_clone, &opus_data, sequence as u32)',
    'dave_guard.encrypt_frame(&sender_id_clone, &opus_data, sequence as u32, &header_buf)'
)

with open('src/player/kizuna_adapter.rs', 'w') as f:
    f.write(adapter)

with open('kizuna-voice/tests/pipeline_test.rs', 'r') as f:
    test = f.read()

test = test.replace(
    'dave_guard.encrypt_frame("sender1", &opus_data, sequence as u32)',
    'dave_guard.encrypt_frame("sender1", &opus_data, sequence as u32, &header_buf)'
)

with open('kizuna-voice/tests/pipeline_test.rs', 'w') as f:
    f.write(test)
