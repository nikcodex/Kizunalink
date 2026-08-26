import re

with open("src/rest/loadtracks.rs", "r") as f:
    content = f.read()

plugin_block = """
    // ----------------------------------------------------
    // DYNAMIC PLUGIN ROUTER
    // ----------------------------------------------------
    if let Some((prefix, query)) = identifier.split_once(':') {
        if let Some(plugin_url) = state.plugin_manager.execute_search(prefix, query) {
            use crate::models::track::{LavalinkTrack, TrackInfo};
            use serde_json::Value;
            
            let mut track = LavalinkTrack {
                encoded: String::new(),
                info: TrackInfo {
                    identifier: format!("{}_{}", prefix, query.replace(" ", "_")),
                    is_seekable: true,
                    author: format!("{} Source", prefix),
                    length: 210000,
                    is_stream: false,
                    position: 0,
                    title: query.to_string(),
                    uri: Some(plugin_url),
                    artwork_url: None,
                    isrc: None,
                    source_name: prefix.to_string(),
                },
                plugin_info: Value::Object(Default::default()),
                user_data: Value::Object(Default::default()),
            };
            
            if let Ok(enc) = crate::track_encoding::encode_track(&track) {
                track.encoded = enc;
            }
            
            return Ok(axum::Json(crate::models::rest::LoadResult::Search(vec![track])));
        }
    }
"""

# Insert right after: let identifier = query.identifier.clone();
content = content.replace("let identifier = query.identifier.clone();", "let identifier = query.identifier.clone();\n" + plugin_block)

with open("src/rest/loadtracks.rs", "w") as f:
    f.write(content)
