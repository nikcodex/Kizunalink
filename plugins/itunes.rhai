print("🍎 iTunes Plugin Triggered!");
let query_encoded = query.replace(" ", "+");
let url = "https://itunes.apple.com/search?term=" + query_encoded + "&entity=song&limit=1";

print("Fetching: " + url);
let json_str = fetch_json(url);

// In Rhai, we can parse JSON using the built-in parse_json if enabled,
// but since we return a raw string URL from the plugin right now, we can extract it manually
// or assume we return the preview URL.
// We'll just extract the previewUrl via a simple string search for the POC.

let index = json_str.index_of("previewUrl\":\"");
if index > 0 {
    let start = index + 13;
    let end = json_str.index_of("\"", start);
    let preview_url = json_str.sub_string(start, end - start);
    print("Found Preview: " + preview_url);
    return preview_url;
}

return "https://itunes.apple.com/error";
