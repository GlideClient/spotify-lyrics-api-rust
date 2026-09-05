use super::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    spotify: Spotify,
    requests: Arc<Mutex<Vec<String>>>,
    server: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture(lyrics_status: u16, rejected_token: bool, first_unauthorized: bool) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let server = tokio::spawn(async move {
        let mut lyrics_calls = 0;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if stream.read(&mut byte).await.unwrap() == 0 {
                    break;
                }
                bytes.push(byte[0]);
            }
            let request = String::from_utf8(bytes).unwrap();
            let path = request.split_whitespace().nth(1).unwrap().to_string();
            captured.lock().unwrap().push(request);
            let (status, body) = if path.starts_with("/time") {
                (200, json!({"serverTime": 1700000000}))
            } else if path.starts_with("/secrets") {
                (200, json!({"9": [0], "61": [1,2,3], "5": [4]}))
            } else if path.starts_with("/token") {
                if rejected_token {
                    (
                        200,
                        json!({"isAnonymous": true, "accessToken": "anonymous"}),
                    )
                } else {
                    (
                        200,
                        json!({"accessToken":"test-access", "accessTokenExpirationTimestampMs":4102444800000u64, "isAnonymous":false}),
                    )
                }
            } else {
                lyrics_calls += 1;
                if first_unauthorized && lyrics_calls == 1 {
                    (401, json!({}))
                } else {
                    (
                        lyrics_status,
                        json!({"lyrics": {"syncType":"LINE_SYNCED", "lines": [
                            {"startTimeMs":"1230", "words":"first", "syllables":["one"], "endTimeMs":"2000"},
                            {"startTimeMs":"2560", "words":"second", "syllables":[], "endTimeMs":"3000"}
                        ]}}),
                    )
                }
            };
            let body = body.to_string();
            let extra = if status == 429 {
                "Retry-After: 17\r\n"
            } else {
                ""
            };
            let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let mut spotify = Spotify::new("test-cookie".into());
    spotify.token_url = format!("{base}/token");
    spotify.server_time_url = format!("{base}/time");
    spotify.secret_key_url = format!("{base}/secrets");
    spotify.lyrics_url = format!("{base}/lyrics/");
    spotify.cache_file = directory.path().join("token.json");
    Fixture {
        spotify,
        requests,
        server,
        _directory: directory,
    }
}

#[test]
fn totp_matches_rfc6238_sha1_vector() {
    assert_eq!(Spotify::generate_totp(59, "12345678901234567890"), "287082");
    assert_eq!(
        Spotify::generate_totp(1111111109, "12345678901234567890"),
        "081804"
    );
}

#[test]
fn selects_latest_numeric_version_and_transforms_decimal_bytes() {
    assert_eq!(
        Spotify::latest_secret(&json!({"9":[0],"61":[1,2,3],"5":[4]})).unwrap(),
        ("888".into(), 61)
    );
    for malformed in [
        json!({}),
        json!({"61":[]}),
        json!({"61":["x"]}),
        json!({"61":[-1]}),
        json!({"61":[256]}),
    ] {
        assert!(Spotify::latest_secret(&malformed).is_err());
    }
}

#[tokio::test]
async fn refresh_uses_upstream_parameters_and_reuses_cached_token() {
    let f = fixture(200, false, false).await;
    f.spotify.get_lyrics("track").await.unwrap();
    f.spotify.get_lyrics("track").await.unwrap();
    let requests = f.requests.lock().unwrap();
    assert_eq!(requests.len(), 5); // time + secrets + token + two lyrics requests
    let token = requests
        .iter()
        .find(|request| request.starts_with("GET /token?"))
        .unwrap();
    let path = token.split_whitespace().nth(1).unwrap();
    let url = url::Url::parse(&format!("http://localhost{path}")).unwrap();
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(params.len(), 5);
    assert_eq!(params["totpVer"], "61");
    assert_eq!(params["totp"], Spotify::generate_totp(1700000000, "888"));
    assert_eq!(params["reason"], "transport");
    assert_eq!(params["productType"], "web-player");
    assert!(params["ts"].parse::<u64>().unwrap() > 1700000000);
    assert!(token.contains("sp_dc=test-cookie"));
    for request in requests
        .iter()
        .filter(|request| !request.starts_with("GET /token?"))
    {
        assert!(!request.contains("test-cookie"));
    }
}

#[tokio::test]
async fn anonymous_token_is_rejected_without_caching_or_lyrics_request() {
    let f = fixture(200, true, false).await;
    assert!(f
        .spotify
        .get_lyrics("track")
        .await
        .unwrap_err()
        .to_string()
        .contains("SP_DC"));
    assert!(!f.spotify.cache_file.exists());
    assert_eq!(f.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn rate_limit_and_not_found_are_preserved() {
    let f = fixture(429, false, false).await;
    assert!(
        matches!(f.spotify.get_lyrics("track").await, Err(SpotifyException::RateLimited { retry_after: Some(value) }) if value == "17")
    );
    let f = fixture(404, false, false).await;
    assert!(matches!(
        f.spotify.get_lyrics("track").await,
        Err(SpotifyException::NotFound)
    ));
}

#[tokio::test]
async fn unauthorized_lyrics_refreshes_once() {
    let f = fixture(200, false, true).await;
    f.spotify.get_lyrics("track").await.unwrap();
    assert_eq!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with("GET /token?"))
            .count(),
        2
    );
}

#[tokio::test]
async fn corrupt_cache_is_replaced() {
    let f = fixture(200, false, false).await;
    std::fs::write(&f.spotify.cache_file, "partial-json").unwrap();
    f.spotify.get_lyrics("track").await.unwrap();
    assert_eq!(
        f.spotify.load_cache_file().unwrap().access_token.as_deref(),
        Some("test-access")
    );
}

#[tokio::test]
async fn output_formats_match_upstream_and_preserve_id3_metadata() {
    let f = fixture(200, false, false).await;
    let id3 = f
        .spotify
        .get_formatted_lyrics("track", "id3")
        .await
        .unwrap();
    assert_eq!(id3["lines"][0]["syllables"], json!(["one"]));
    assert_eq!(id3["lines"][0]["endTimeMs"], "2000");
    let lrc = f
        .spotify
        .get_formatted_lyrics("track", "lrc")
        .await
        .unwrap();
    assert_eq!(lrc["lines"][0]["timeTag"], "00:01.23");
    let srt = f
        .spotify
        .get_formatted_lyrics("track", "srt")
        .await
        .unwrap();
    assert_eq!(
        srt["lines"],
        json!([{"index":1,"startTime":"00:00:01,230","endTime":"00:00:02,560","words":"first"}])
    );
    let raw = f
        .spotify
        .get_formatted_lyrics("track", "raw")
        .await
        .unwrap();
    assert_eq!(raw["lines"], "first\nsecond\n");
}

#[tokio::test]
async fn expired_cached_token_is_refreshed_before_requesting_lyrics() {
    let f = fixture(200, false, false).await;
    std::fs::write(
        &f.spotify.cache_file,
        r#"{"access_token":"expired","access_token_expiration_timestamp_ms":0}"#,
    )
    .unwrap();
    f.spotify.get_lyrics("track").await.unwrap();
    let requests = f.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    let lyrics = requests
        .iter()
        .find(|r| r.starts_with("GET /lyrics/"))
        .unwrap();
    assert!(lyrics.contains("Bearer test-access"));
    assert!(!lyrics.contains("Bearer expired"));
}

#[tokio::test]
async fn repeated_unauthorized_stops_after_one_refresh() {
    let f = fixture(401, false, false).await;
    assert!(f.spotify.get_lyrics("track").await.is_err());
    let requests = f.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.starts_with("GET /token?"))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.starts_with("GET /lyrics/"))
            .count(),
        2
    );
}

#[tokio::test]
async fn incomplete_token_response_is_never_cached() {
    let mut f = fixture(200, false, false).await;
    f.spotify.token_url = f.spotify.token_url.replace("/token", "/bad-token");
    assert!(f
        .spotify
        .get_token()
        .await
        .unwrap_err()
        .to_string()
        .contains("missing accessToken"));
    assert!(!f.spotify.cache_file.exists());
}

#[tokio::test]
#[ignore = "Requires a valid SP_DC environment variable and contacts Spotify"]
async fn live_authorization_and_lyrics() {
    let cookie = std::env::var("SP_DC").expect("SP_DC is required for this opt-in test");
    let directory = tempfile::tempdir().unwrap();
    let mut spotify = Spotify::new(cookie);
    spotify.cache_file = directory.path().join("token.json");
    spotify
        .get_token()
        .await
        .expect("Live authorization failed");
    println!("Live authorization succeeded");
    let result = spotify
        .get_formatted_lyrics("0VjIjW4GlUZAMYd2vXMi3b", "id3")
        .await;
    match result {
        Ok(lyrics) => {
            let lines = lyrics["lines"].as_array().expect("Lyrics lines missing");
            assert!(!lines.is_empty());
            println!(
                "Live authorization succeeded; received {} lyric lines ({})",
                lines.len(),
                lyrics["syncType"]
            );
        }
        Err(error) => panic!("Live lyrics test failed: {error}"),
    }
}
