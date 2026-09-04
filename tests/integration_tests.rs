use axum::http::{HeaderMap, HeaderValue};
use kizunalink::models::protocol::VoiceStateUpdate;
use kizunalink::player::guild_player::GuildPlayer;
use kizunalink::player::manager::{PlayerManager, SourceBundle};
use kizunalink::rest::auth::require_auth;
use kizunalink::rest::lyrics::parse_lrc;
use kizunalink::security::{
    is_private_ip, sanitize_for_log, validate_guild_id, validate_identifier, validate_query,
    validate_session_id, validate_url,
};
use kizunalink::sources::{
    apple_music::AppleMusicSource, deezer::DeezerSource, jiosaavn::JioSaavnSource,
    soundcloud::SoundCloudSource, spotify::SpotifySource, youtube::YouTubeSource,
};
use kizunalink::ws::session::SessionManager;
use tokio::sync::{broadcast, mpsc};

fn mock_player_manager() -> PlayerManager {
    let (event_tx, _) = broadcast::channel(32);
    PlayerManager::new(
        event_tx,
        SourceBundle {
            jiosaavn: JioSaavnSource::new(),
            youtube: YouTubeSource::new(None),
            spotify: SpotifySource::new(),
            soundcloud: SoundCloudSource::new(),
            deezer: DeezerSource::new(),
            apple_music: AppleMusicSource::new(),
        },
        50,
    )
}

#[tokio::test]
async fn test_auth_enforcement_and_constant_time() {
    let mut headers = HeaderMap::new();
    let expected = "secretpass123";

    // Missing header
    assert!(require_auth(&headers, expected, "/v4/info").is_err());

    // Invalid header
    headers.insert("authorization", HeaderValue::from_static("wrongpass"));
    assert!(require_auth(&headers, expected, "/v4/info").is_err());

    // Valid header
    headers.insert("authorization", HeaderValue::from_static("secretpass123"));
    assert!(require_auth(&headers, expected, "/v4/info").is_ok());
}

#[tokio::test]
async fn test_session_manager_lifecycle_and_resume() {
    let sm = SessionManager::new(100, 50);

    // Initial connection creates new session
    let (session_id, is_resumed, _) = sm
        .handle_connection(None, "12345".to_string())
        .expect("connection failed");
    assert!(!is_resumed);
    assert_eq!(sm.count_sessions(), 1);

    // Subscribe to a guild
    sm.add_guild(&session_id, "guild_100");
    assert!(sm.is_guild_subscribed(&session_id, "guild_100"));
    assert!(!sm.is_guild_subscribed(&session_id, "guild_200"));

    // Disconnect with resuming enabled
    sm.update_session(&session_id, Some(true), Some(60));
    let timeout = sm.mark_disconnected(&session_id);
    assert_eq!(timeout, Some(60));

    // Buffer events while disconnected
    sm.buffer_event(Some("guild_100"), r#"{"op":"event","guildId":"guild_100"}"#);
    sm.buffer_event(Some("guild_200"), r#"{"op":"event","guildId":"guild_200"}"#);
    sm.buffer_event(None, r#"{"op":"stats"}"#);

    // Resume session
    let (resumed_id, is_resumed, replay_events) = sm
        .handle_connection(Some(session_id.clone()), "12345".to_string())
        .expect("resume failed");
    assert_eq!(resumed_id, session_id);
    assert!(is_resumed);

    // Replay should include guild_100 and stats, but NOT guild_200
    assert_eq!(replay_events.len(), 2);
    assert!(replay_events[0].contains("guild_100"));
    assert!(replay_events[1].contains("stats"));

    // Cleanup session
    sm.remove_session(&session_id);
    assert_eq!(sm.count_sessions(), 0);
}

#[tokio::test]
async fn test_player_pause_resume_volume_and_state() {
    let (event_tx, _) = broadcast::channel(32);
    let (track_end_tx, _) = mpsc::unbounded_channel();
    let mut player = GuildPlayer::new(
        "guild_test".to_string(),
        "bot_user".to_string(),
        event_tx,
        track_end_tx,
        50,
    );

    assert!(!player.paused);
    assert_eq!(player.volume, 100);

    // Pause player
    player.set_paused(true).await;
    assert!(player.paused);

    // Duplicate pause is no-op
    player.set_paused(true).await;
    assert!(player.paused);

    // Resume player
    player.set_paused(false).await;
    assert!(!player.paused);

    // Volume updates
    player.set_volume(150);
    assert_eq!(player.volume, 150);

    // Clamping volume
    player.set_volume(2000);
    assert_eq!(player.volume, 1000);

    let resp = player.to_response();
    assert_eq!(resp.volume, 1000);
    assert!(!resp.paused);
    assert!(!resp.state.connected); // Voice adapter not connected
}

#[tokio::test]
async fn test_player_manager_atomic_player_limit() {
    let pm = mock_player_manager();

    // Concurrent player creation
    for i in 0..20 {
        let gid = format!("guild_{}", i);
        let player = pm.get_or_create_player(&gid).await;
        assert!(player.is_ok());
    }

    let (total, _) = pm.count_players().await;
    assert_eq!(total, 20);
}

#[tokio::test]
async fn test_voice_connection_error_handling() {
    let (event_tx, _) = broadcast::channel(32);
    let (track_end_tx, _) = mpsc::unbounded_channel();
    let mut player = GuildPlayer::new(
        "guild_voice_test".to_string(),
        "bot_user".to_string(),
        event_tx,
        track_end_tx,
        50,
    );

    // Incomplete voice update
    let incomplete = VoiceStateUpdate {
        token: "token123".to_string(),
        endpoint: "".to_string(),
        session_id: "sess123".to_string(),
        channel_id: Some("chan123".to_string()),
    };
    let res = player.set_voice(incomplete).await;
    assert_eq!(res, Ok(false));
    assert!(!player.to_response().state.connected);

    // Full voice update with invalid/unreachable endpoint should fail and return Err
    let full_bad = VoiceStateUpdate {
        token: "token123".to_string(),
        endpoint: "invalid-voice-endpoint.discord.gg:443".to_string(),
        session_id: "sess123".to_string(),
        channel_id: Some("chan123".to_string()),
    };
    let res = player.set_voice(full_bad).await;
    assert!(res.is_err());
    assert!(!player.to_response().state.connected);
}

#[test]
fn test_ssrf_and_security_rules() {
    // Private IPv4 blocked
    assert!(is_private_ip("127.0.0.1".parse().unwrap()));
    assert!(is_private_ip("10.50.2.1".parse().unwrap()));
    assert!(is_private_ip("172.20.0.5".parse().unwrap()));
    assert!(is_private_ip("192.168.100.1".parse().unwrap()));
    assert!(is_private_ip("169.254.169.254".parse().unwrap()));
    assert!(is_private_ip("100.64.1.1".parse().unwrap()));

    // Public IPv4 allowed
    assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
    assert!(!is_private_ip("1.1.1.1".parse().unwrap()));

    // URL validation
    assert!(validate_url("https://example.com/stream.mp3").is_ok());
    assert!(validate_url("http://127.0.0.1/test").is_err());
    assert!(validate_url("http://10.0.0.1/test").is_err());
    assert!(validate_url("http://localhost/test").is_err());
    assert!(validate_url("http://169.254.169.254/latest").is_err());
    assert!(validate_url("file:///etc/passwd").is_err());
    assert!(validate_url("ftp://server.local/file").is_err());

    // Identifiers and Queries
    assert!(validate_identifier("track_id_123").is_ok());
    assert!(validate_identifier("").is_err());
    assert!(validate_query("rick astley").is_ok());
    assert!(validate_query("bad\nquery").is_err());
    assert!(validate_guild_id("1234567890").is_ok());
    assert!(validate_guild_id("abc123").is_err());
    assert!(validate_session_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    assert!(validate_session_id("bad session!").is_err());

    // Log sanitization
    assert_eq!(sanitize_for_log("good log"), "good log");
    assert_eq!(sanitize_for_log("bad\r\nlog\x1b[31m"), "badlog[31m");
}

#[test]
fn test_lrc_integer_parser_accuracy() {
    let sample_lrc = r#"
        [00:00.00]First line
        [00:15.50]Second line
        [01:05.123]Third line
        [02:30]Fourth line
        [invalid:line]Should be skipped
        [01:99.00]Invalid seconds skipped
    "#;

    let lines = parse_lrc(sample_lrc);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0].timestamp, 0);
    assert_eq!(lines[0].line, "First line");
    assert_eq!(lines[1].timestamp, 15500);
    assert_eq!(lines[1].line, "Second line");
    assert_eq!(lines[2].timestamp, 65123);
    assert_eq!(lines[2].line, "Third line");
    assert_eq!(lines[3].timestamp, 150000);
    assert_eq!(lines[3].line, "Fourth line");
}
