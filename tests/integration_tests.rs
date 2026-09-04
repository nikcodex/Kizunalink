use axum::http::{HeaderMap, HeaderValue};
use kizunalink::models::protocol::{
    PlayerResponse, PlayerState, PlayerUpdatePayload, VoiceStateUpdate,
};
use kizunalink::models::track::{ErrorInfo, LavalinkTrack, TrackInfo};
use kizunalink::player::guild_player::GuildPlayer;
use kizunalink::player::manager::{PlayerManager, SourceBundle, MAX_PLAYERS};
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
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

fn mock_player_manager() -> PlayerManager {
    mock_player_manager_with_limit(MAX_PLAYERS)
}

fn mock_player_manager_with_limit(max_players: usize) -> PlayerManager {
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
        max_players,
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
    assert_eq!(
        sm.update_session(&session_id, Some(true), Some(60)),
        Some((true, 60))
    );
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
        endpoint: "127.0.0.1:1".to_string(),
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

#[tokio::test]
async fn test_session_isolation_and_guild_tracking() {
    let sm = SessionManager::new(100, 50);

    let (session_a, _, _) = sm.handle_connection(None, "user_a".to_string()).unwrap();
    let (session_b, _, _) = sm.handle_connection(None, "user_b".to_string()).unwrap();

    // Subscribe session A to guild 100
    sm.add_guild(&session_a, "guild_100");

    assert!(sm.is_guild_subscribed(&session_a, "guild_100"));
    assert!(!sm.is_guild_subscribed(&session_b, "guild_100"));

    let a_guilds = sm.get_session_guilds(&session_a);
    let b_guilds = sm.get_session_guilds(&session_b);

    assert!(a_guilds.contains("guild_100"));
    assert!(!b_guilds.contains("guild_100"));
}

#[test]
fn test_load_result_empty_json_matches_lavalink_v4() {
    let empty_result = kizunalink::models::track::LoadResult::Empty;
    let json_str = serde_json::to_string(&empty_result).expect("serialization failed");
    assert_eq!(json_str, r#"{"loadType":"empty","data":null}"#);
}

// ---------------------------------------------------------------------------
// Session ownership isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_player_ownership_isolation() {
    let pm = mock_player_manager();
    let payload = PlayerUpdatePayload {
        volume: Some(80),
        ..Default::default()
    };

    // Session A creates the player for guild_100
    assert!(pm
        .update_player("guild_100", payload.clone(), false, "session_a")
        .await
        .is_ok());

    // Session B must not read, update, list, or destroy A's player
    assert!(pm
        .get_player_for_session("guild_100", "session_b")
        .await
        .is_none());
    assert!(pm
        .update_player("guild_100", payload.clone(), false, "session_b")
        .await
        .is_err());
    assert!(pm
        .destroy_player_for_session("guild_100", "session_b")
        .is_err());
    assert!(pm.get_players_for_session("session_b").await.is_empty());
    assert_eq!(pm.get_players_for_session("session_a").await.len(), 1);

    // Session A still fully controls its player
    let resp = pm.get_player_for_session("guild_100", "session_a").await;
    assert!(resp.is_some());
    assert_eq!(resp.unwrap().volume, 80);

    // Session B can create its own player for a different guild
    assert!(pm
        .update_player("guild_200", payload.clone(), false, "session_b")
        .await
        .is_ok());
    // ... and session A cannot touch it
    assert!(pm
        .update_player(
            "guild_200",
            PlayerUpdatePayload::default(),
            false,
            "session_a"
        )
        .await
        .is_err());
    assert!(pm
        .get_player_for_session("guild_200", "session_a")
        .await
        .is_none());

    // Destroy by the owner works; afterwards the guild is gone for everyone
    assert!(pm
        .destroy_player_for_session("guild_100", "session_a")
        .is_ok());
    assert!(pm
        .get_player_for_session("guild_100", "session_a")
        .await
        .is_none());
    assert!(
        pm.update_player("guild_100", payload, false, "session_b")
            .await
            .is_ok(),
        "after destruction a new session may claim the guild"
    );
}

#[tokio::test]
async fn test_concurrent_player_ownership_claim_single_owner() {
    let pm = Arc::new(mock_player_manager());
    let mut tasks = Vec::new();
    for i in 0..16 {
        let pm = pm.clone();
        tasks.push(tokio::spawn(async move {
            let session = if i % 2 == 0 { "session_a" } else { "session_b" };
            pm.update_player("guild_race", PlayerUpdatePayload::default(), false, session)
                .await
        }));
    }

    let mut a_ok = 0usize;
    let mut b_ok = 0usize;
    for (i, task) in tasks.into_iter().enumerate() {
        let session = if i % 2 == 0 { "session_a" } else { "session_b" };
        match task.await.unwrap() {
            Ok(_) if session == "session_a" => a_ok += 1,
            Ok(_) => b_ok += 1,
            Err(_) => {}
        }
    }

    // Exactly one session wins the guild — all successes share a single owner.
    assert_eq!((a_ok > 0) as usize + (b_ok > 0) as usize, 1);
    assert_eq!(a_ok + b_ok, 1);
    let (total, _) = pm.count_players().await;
    assert_eq!(total, 1);
}

#[tokio::test]
async fn test_concurrent_player_limit_enforced_atomically() {
    let pm = Arc::new(mock_player_manager_with_limit(5));
    let mut tasks = Vec::new();
    for i in 0..50 {
        let pm = pm.clone();
        tasks.push(tokio::spawn(async move {
            let gid = format!("guild_limit_{}", i);
            pm.update_player(&gid, PlayerUpdatePayload::default(), false, "session_a")
                .await
                .is_ok()
        }));
    }

    let mut ok_count = 0;
    for task in tasks {
        if task.await.unwrap() {
            ok_count += 1;
        }
    }
    assert!(ok_count <= 5, "{} creations succeeded", ok_count);
    let (total, _) = pm.count_players().await;
    assert_eq!(total, ok_count);
}

// ---------------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unknown_session_update_never_creates_session() {
    let sm = SessionManager::new(100, 50);
    assert_eq!(sm.count_sessions(), 0);

    // PATCHing an unknown session must not create it
    assert!(sm
        .update_session("550e8400-e29b-41d4-a716-446655440000", Some(true), Some(60))
        .is_none());
    assert_eq!(sm.count_sessions(), 0);

    // Legitimate sessions still update
    let (sid, _, _) = sm.handle_connection(None, "user".to_string()).unwrap();
    assert_eq!(
        sm.update_session(&sid, Some(true), Some(120)),
        Some((true, 120))
    );
    assert_eq!(
        sm.update_session(&sid, Some(false), None),
        Some((false, 120))
    );
}

#[tokio::test]
async fn test_session_connection_limit_max_connections() {
    let sm = SessionManager::new(2, 50);
    assert!(sm.handle_connection(None, "a".to_string()).is_ok());
    assert!(sm.handle_connection(None, "b".to_string()).is_ok());
    assert!(sm.handle_connection(None, "c".to_string()).is_err());
    assert_eq!(sm.count_sessions(), 2);
}

#[test]
fn test_session_expiration_exact_timeout_semantics() {
    use kizunalink::ws::session::SessionState;
    use std::time::{Duration, Instant};

    let now = Instant::now();

    let mut fresh = SessionState::default();
    fresh.connected = false;
    fresh.resuming = true;
    fresh.timeout = 60;
    fresh.last_active = now - Duration::from_secs(30);
    assert!(
        !SessionManager::is_session_expired(&fresh, now),
        "30s elapsed with 60s timeout must not expire"
    );

    let mut expired = fresh.clone();
    expired.last_active = now - Duration::from_secs(61);
    assert!(
        SessionManager::is_session_expired(&expired, now),
        "61s elapsed with 60s timeout must expire (exact timeout, not doubled)"
    );

    // timeout = 0: disconnected session expires immediately (no resume window)
    let mut zero_timeout = fresh.clone();
    zero_timeout.timeout = 0;
    assert!(SessionManager::is_session_expired(&zero_timeout, now));

    // Connected sessions never expire, even with an old last_active
    let mut connected = fresh.clone();
    connected.connected = true;
    connected.last_active = now - Duration::from_secs(3600);
    assert!(!SessionManager::is_session_expired(&connected, now));

    // Non-resuming disconnected sessions are immediately expired
    let mut no_resume = fresh.clone();
    no_resume.resuming = false;
    assert!(SessionManager::is_session_expired(&no_resume, now));
}

// ---------------------------------------------------------------------------
// Wire format (Lavalink camelCase JSON)
// ---------------------------------------------------------------------------

#[test]
fn test_wire_format_camelcase_track_info() {
    let track = LavalinkTrack {
        encoded: "enc".to_string(),
        info: TrackInfo {
            identifier: "dQw4w9WgXcQ".to_string(),
            is_seekable: true,
            author: "Rick".to_string(),
            length: 212000,
            is_stream: false,
            position: 1500,
            title: "Never Gonna Give You Up".to_string(),
            uri: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string()),
            artwork_url: Some("https://i.ytimg.com/vi/x.jpg".to_string()),
            isrc: Some("GBARL8700014".to_string()),
            source_name: "youtube".to_string(),
        },
        plugin_info: serde_json::json!({}),
        user_data: serde_json::json!({}),
    };

    let v = serde_json::to_value(&track).unwrap();
    let obj = v.as_object().unwrap();
    for key in ["encoded", "info", "pluginInfo", "userData"] {
        assert!(obj.contains_key(key), "missing {}", key);
    }
    assert!(!obj.contains_key("plugin_info"));
    assert!(!obj.contains_key("user_data"));

    let info = obj["info"].as_object().unwrap();
    for key in [
        "identifier",
        "isSeekable",
        "author",
        "length",
        "isStream",
        "position",
        "title",
        "uri",
        "artworkUrl",
        "isrc",
        "sourceName",
    ] {
        assert!(info.contains_key(key), "missing {}", key);
    }
    for snake in ["is_seekable", "is_stream", "artwork_url", "source_name"] {
        assert!(!info.contains_key(snake), "snake_case {} leaked", snake);
    }
}

#[test]
fn test_wire_format_camelcase_player_response() {
    let track = LavalinkTrack {
        encoded: "enc".to_string(),
        info: TrackInfo {
            identifier: "id".to_string(),
            is_seekable: true,
            author: "A".to_string(),
            length: 1000,
            is_stream: false,
            position: 0,
            title: "T".to_string(),
            uri: Some("https://example.com".to_string()),
            artwork_url: None,
            isrc: None,
            source_name: "youtube".to_string(),
        },
        plugin_info: serde_json::json!({}),
        user_data: serde_json::json!({}),
    };
    let resp = PlayerResponse {
        guild_id: "123456789".to_string(),
        track: Some(track),
        volume: 100,
        paused: false,
        state: PlayerState {
            time: 1,
            position: 2,
            connected: true,
            ping: 3,
        },
        voice: VoiceStateUpdate {
            token: "tok".to_string(),
            endpoint: "endpoint".to_string(),
            session_id: "sess".to_string(),
            channel_id: Some("chan".to_string()),
        },
        filters: kizunalink::models::filters::Filters::default(),
        autoplay: false,
        loop_mode: "none".to_string(),
        is_playing: false,
    };

    let v = serde_json::to_value(&resp).unwrap();
    let obj = v.as_object().unwrap();
    for key in [
        "guildId", "track", "volume", "paused", "state", "voice", "filters", "autoplay", "loop",
    ] {
        assert!(obj.contains_key(key), "missing {}", key);
    }
    for snake in [
        "guild_id",
        "loop_mode",
        "is_playing",
        "sessionId_in_voice_wrong_place",
    ] {
        assert!(!obj.contains_key(snake), "snake_case {} leaked", snake);
    }
    let voice = obj["voice"].as_object().unwrap();
    assert!(voice.contains_key("sessionId"));
    assert!(voice.contains_key("channelId"));
    assert!(!voice.contains_key("session_id"));
}

#[test]
fn test_wire_format_error_info_and_payloads() {
    // ErrorInfo uses causeStackTrace
    let err = serde_json::to_value(ErrorInfo {
        message: Some("boom".to_string()),
        severity: "fault".to_string(),
        cause: "cause".to_string(),
        cause_stack_trace: "at x".to_string(),
    })
    .unwrap();
    let obj = err.as_object().unwrap();
    assert!(obj.contains_key("causeStackTrace"));
    assert!(!obj.contains_key("cause_stack_trace"));

    // PlayerUpdatePayload deserializes Lavalink's camelCase body
    let json = r#"{
        "track": {"encoded": "abc", "userData": {"x": 1}},
        "position": 500,
        "endTime": 10000,
        "volume": 75,
        "paused": false,
        "filters": {"volume": 0.5},
        "voice": {"token": "t", "endpoint": "e", "sessionId": "s", "channelId": "c"}
    }"#;
    let p: PlayerUpdatePayload = serde_json::from_str(json).expect("camelCase PATCH body");
    assert_eq!(p.end_time, Some(10000));
    assert_eq!(p.position, Some(500));
    assert_eq!(p.volume, Some(75));
    assert_eq!(p.voice.as_ref().unwrap().session_id, "s");
    assert_eq!(
        p.track.as_ref().unwrap().user_data.as_ref().unwrap()["x"],
        1
    );
    assert!(p.filters.is_some());

    // NoReplace query param parses with the Lavalink spelling
    let q: kizunalink::rest::players::NoReplaceQuery =
        serde_json::from_str(r#"{"noReplace": true}"#).unwrap();
    assert_eq!(q.no_replace, Some(true));
}

// ---------------------------------------------------------------------------
// Track URI semantics
// ---------------------------------------------------------------------------

#[test]
fn test_http_track_uri_is_canonical_source_url() {
    let track = kizunalink::util::create_http_track("https://cdn.example.com/song.mp3");
    // The canonical URI is the source URL — never a derived/internal stream URL.
    assert_eq!(
        track.info.uri.as_deref(),
        Some("https://cdn.example.com/song.mp3")
    );
    assert_eq!(track.info.identifier, "https://cdn.example.com/song.mp3");
    assert_eq!(track.info.source_name, "http");
}

#[test]
fn test_encoded_track_uri_stays_canonical() {
    // Round-trip a track whose pluginInfo carries a resolved stream URL: the
    // public uri must remain the canonical source URL.
    let track = LavalinkTrack {
        encoded: String::new(),
        info: TrackInfo {
            identifier: "dQw4w9WgXcQ".to_string(),
            is_seekable: true,
            author: "Rick".to_string(),
            length: 212000,
            is_stream: false,
            position: 0,
            title: "Never Gonna Give You Up".to_string(),
            uri: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string()),
            artwork_url: None,
            isrc: None,
            source_name: "youtube".to_string(),
        },
        plugin_info: serde_json::json!({"streamUrl": "https://rr1.googlevideo.com/videoplayback?x=1"}),
        user_data: serde_json::json!({}),
    };
    let encoded = kizunalink::track_encoding::encode_track(&track).unwrap();
    let decoded = kizunalink::track_encoding::decode_track(&encoded).unwrap();
    assert_eq!(
        decoded.info.uri.as_deref(),
        Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
    );
    assert_eq!(decoded.info.source_name, "youtube");
}
