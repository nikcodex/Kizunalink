// [LOCAL INTEGRATION]
use kizunalink::models::track::{LavalinkTrack, TrackInfo};
use kizunalink::player::queue::{LoopMode, TrackQueue};

fn make_test_track(title: &str, identifier: &str) -> LavalinkTrack {
    LavalinkTrack {
        encoded: format!("encoded_{}", identifier),
        info: TrackInfo {
            identifier: identifier.to_string(),
            is_seekable: true,
            author: "Test Author".to_string(),
            length: 180000,
            is_stream: false,
            position: 0,
            title: title.to_string(),
            uri: Some(format!("https://example.com/{}", identifier)),
            artwork_url: None,
            isrc: None,
            source_name: "test".to_string(),
        },
        plugin_info: serde_json::Value::Null,
        user_data: serde_json::Value::Null,
    }
}

#[test]
fn test_queue_advancement_natural_flow() {
    let mut queue = TrackQueue::new();
    let track_a = make_test_track("Track A", "track_a");
    let track_b = make_test_track("Track B", "track_b");
    let track_c = make_test_track("Track C", "track_c");

    queue.add(track_a.clone());
    queue.add(track_b.clone());
    queue.add(track_c.clone());

    assert_eq!(queue.len(), 3);

    // Start playing Track A
    let current_1 = queue.next().expect("Track A starts");
    assert_eq!(current_1.info.title, "Track A");
    queue.current = Some(current_1);

    // Track A finishes -> Track B starts
    let current_2 = queue.next().expect("Track B starts");
    assert_eq!(current_2.info.title, "Track B");
    queue.current = Some(current_2);

    // Track B finishes -> Track C starts
    let current_3 = queue.next().expect("Track C starts");
    assert_eq!(current_3.info.title, "Track C");
    queue.current = Some(current_3);

    // Track C finishes -> Queue is empty
    let current_4 = queue.next();
    assert!(current_4.is_none(), "Queue must be exhausted");
}

#[test]
fn test_queue_track_loop_mode() {
    let mut queue = TrackQueue::new();
    let track_a = make_test_track("Track A", "track_a");
    let track_b = make_test_track("Track B", "track_b");

    queue.add(track_a.clone());
    queue.add(track_b.clone());
    queue.set_loop(LoopMode::Track);

    // Initial pop starts Track A
    let first = queue.next().expect("First track");
    assert_eq!(first.info.title, "Track A");
    queue.current = Some(first);

    // In LoopMode::Track, next() replays current track
    let repeat_1 = queue.next().expect("Repeat 1");
    assert_eq!(repeat_1.info.title, "Track A");

    let repeat_2 = queue.next().expect("Repeat 2");
    assert_eq!(repeat_2.info.title, "Track A");

    // Disable loop -> advances to Track B
    queue.set_loop(LoopMode::None);
    let next_track = queue.next().expect("Next track");
    assert_eq!(next_track.info.title, "Track B");
}

#[test]
fn test_queue_stop_and_clear_no_duplicate_advancement() {
    let mut queue = TrackQueue::new();
    let track_a = make_test_track("Track A", "track_a");
    let track_b = make_test_track("Track B", "track_b");

    queue.add(track_a);
    queue.add(track_b);

    // Start Track A
    let cur = queue.next().unwrap();
    queue.current = Some(cur);

    // Stop and clear queue
    queue.clear();
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);

    // Calling next on cleared queue returns None
    assert!(queue.next().is_none());
}
