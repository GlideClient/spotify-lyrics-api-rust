use crate::spotifyexception::SpotifyException;
use hmac::{Hmac, Mac};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, SpotifyException>;

#[derive(Serialize, Deserialize, Debug)]
struct CacheData {
    #[serde(skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    access_token_expiration_timestamp_ms: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LrcLine {
    #[serde(rename = "timeTag")]
    pub time_tag: String,
    pub words: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LrcResponse {
    pub error: bool,
    #[serde(rename = "syncType")]
    pub sync_type: String,
    pub lines: Vec<LrcLine>,
}

pub struct Spotify {
    token_url: String,
    lyrics_url: String,
    server_time_url: String,
    secret_key_url: String,
    client: reqwest::Client,
    sp_dc: String,
    cache_file: PathBuf,
}

impl Spotify {
    /// Create a new Spotify instance with the provided sp_dc cookie value
    pub fn new(sp_dc: String) -> Self {
        let cache_file = std::env::temp_dir().join("spotify_token.json");

        Spotify {
            token_url: "https://open.spotify.com/api/token".to_string(),
            lyrics_url: "https://spclient.wg.spotify.com/color-lyrics/v2/track/".to_string(),
            server_time_url: "https://open.spotify.com/api/server-time".to_string(),
            secret_key_url: "https://raw.githubusercontent.com/xyloflake/spot-secrets-go/main/secrets/secretDict.json".to_string(),
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
                .build().expect("Failed to initialize HTTP client"),
            sp_dc,
            cache_file,
        }
    }

    /// Loads the cache file and returns the data
    fn load_cache_file(&self) -> Result<CacheData> {
        if self.cache_file.exists() {
            let mut file = File::open(&self.cache_file)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;

            let data = serde_json::from_str(&contents)?;
            Ok(data)
        } else {
            Ok(CacheData {
                access_token: None,
                client_id: None,
                access_token_expiration_timestamp_ms: None,
            })
        }
    }

    /// Saves the cache data to the cache file
    fn save_cache_file(&self, data: &CacheData) -> Result<()> {
        // Publish complete files so another reader cannot see a partially written token.
        let mut file = tempfile::NamedTempFile::new_in(
            self.cache_file
                .parent()
                .unwrap_or_else(|| std::path::Path::new(".")),
        )?;
        serde_json::to_writer(&mut file, data)?;
        file.flush()?;
        file.persist(&self.cache_file)
            .map_err(|error| error.error)?;
        Ok(())
    }

    /// Match upstream's rotating secret dictionary and decimal XOR transformation.
    fn latest_secret(data: &serde_json::Value) -> Result<(String, u64)> {
        let entries = data
            .as_object()
            .ok_or_else(|| SpotifyException::new("Invalid TOTP secret dictionary"))?;
        let (version, value) = entries
            .iter()
            .filter_map(|(version, value)| {
                version.parse::<u64>().ok().map(|version| (version, value))
            })
            .max_by_key(|(version, _)| *version)
            .ok_or_else(|| SpotifyException::new("No TOTP secret version found"))?;
        let values = value
            .as_array()
            .filter(|values| !values.is_empty())
            .ok_or_else(|| {
                SpotifyException::new("The original secret must be a nonempty array of integers")
            })?;
        let mut secret = String::new();
        for (index, value) in values.iter().enumerate() {
            let value = value
                .as_u64()
                .filter(|value| *value <= 255)
                .ok_or_else(|| SpotifyException::new("Invalid TOTP secret value"))?;
            secret.push_str(&(value ^ ((index as u64 % 33) + 9)).to_string());
        }
        Ok((secret, version))
    }

    async fn get_latest_secret(&self) -> Result<(String, u64)> {
        let response = self.client.get(&self.secret_key_url).send().await?;
        Self::check_status(&response, "Secret dictionary request")?;
        Self::latest_secret(&response.json().await?)
    }

    fn check_status(response: &reqwest::Response, operation: &str) -> Result<()> {
        let status = response.status();
        if status.as_u16() == 429 {
            return Err(SpotifyException::RateLimited {
                retry_after: response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
            });
        }
        if !status.is_success() {
            return Err(SpotifyException::ApiError(format!(
                "{} failed: HTTP status {}",
                operation, status
            )));
        }
        Ok(())
    }

    /// Generates a Time-based One-Time Password (TOTP) using the server time
    fn generate_totp(server_time_seconds: u64, secret: &str) -> String {
        // Calculate the counter value (number of time steps since epoch)
        let time_step = 30; // seconds
        let counter = server_time_seconds / time_step;

        // Create a byte array for the counter (8 bytes, big-endian)
        let counter_bytes = counter.to_be_bytes();

        // Calculate HMAC-SHA1
        let mut mac =
            Hmac::<Sha1>::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(&counter_bytes);
        let result = mac.finalize().into_bytes();

        // Dynamic truncation
        let offset = (result[19] & 0xf) as usize;
        let binary = ((result[offset] & 0x7f) as u32) << 24
            | (result[offset + 1] as u32) << 16
            | (result[offset + 2] as u32) << 8
            | (result[offset + 3] as u32);

        // Generate 6-digit code
        let otp = binary % 1_000_000;
        format!("{:06}", otp)
    }

    /// Retrieves the server time and returns the parameters needed for the token request
    async fn get_server_time_params(&self) -> Result<HashMap<String, String>> {
        let response = self.client.get(&self.server_time_url).send().await?;
        Self::check_status(&response, "Server time request")?;

        let server_time_data: serde_json::Value = response.json().await?;

        let server_time_seconds = server_time_data["serverTime"]
            .as_u64()
            .ok_or_else(|| SpotifyException::new("Invalid server time response"))?;

        let (secret, version) = self.get_latest_secret().await?;
        let totp = Self::generate_totp(server_time_seconds, &secret);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SpotifyException::new("System clock is before the Unix epoch"))?
            .as_secs();

        let mut params = HashMap::new();
        params.insert("reason".to_string(), "transport".to_string());
        params.insert("productType".to_string(), "web-player".to_string());
        params.insert("totp".to_string(), totp);
        params.insert("totpVer".to_string(), version.to_string());
        params.insert("ts".to_string(), timestamp.to_string());

        Ok(params)
    }

    /// Retrieves an access token from Spotify and stores it in a file
    pub async fn get_token(&self) -> Result<()> {
        if self.sp_dc.is_empty() {
            return Err(SpotifyException::new(
                "Please set SP_DC as an environmental variable.",
            ));
        }

        let params = self.get_server_time_params().await?;
        let response = self
            .client
            .get(&self.token_url)
            .query(&params)
            .header("Cookie", format!("sp_dc={}", self.sp_dc))
            .send()
            .await?;
        Self::check_status(&response, "Token request")?;

        let token_json: serde_json::Value = response.json().await?;

        // Check if token is anonymous (invalid sp_dc)
        if token_json
            .get("isAnonymous")
            .is_some_and(|v| v.as_bool().unwrap_or(false))
        {
            return Err(SpotifyException::new(
                "The SP_DC set seems to be invalid, please correct it!",
            ));
        }

        let mut cache_data = CacheData {
            access_token: None,
            client_id: None,
            access_token_expiration_timestamp_ms: None,
        };

        cache_data.access_token = token_json["accessToken"].as_str().map(String::from);
        cache_data.access_token_expiration_timestamp_ms =
            token_json["accessTokenExpirationTimestampMs"].as_u64();

        // Keep the optional client ID from this token response.
        if let Some(client_id) = token_json["clientId"].as_str() {
            cache_data.client_id = Some(client_id.to_string());
        }

        if cache_data
            .access_token
            .as_ref()
            .is_none_or(|token| token.is_empty())
            || cache_data.access_token_expiration_timestamp_ms.is_none()
        {
            return Err(SpotifyException::new(
                "Token response is missing accessToken or accessTokenExpirationTimestampMs",
            ));
        }
        self.save_cache_file(&cache_data)?;

        Ok(())
    }

    /// Checks if the access token and client token are expired and retrieves new ones if needed
    async fn check_tokens_expire(&self) -> Result<()> {
        let cache_exists = self.cache_file.exists();

        let cache_data = if cache_exists {
            self.load_cache_file().unwrap_or(CacheData {
                access_token: None,
                client_id: None,
                access_token_expiration_timestamp_ms: None,
            })
        } else {
            debug!("No token cache file found, creating new one");
            CacheData {
                access_token: None,
                client_id: None,
                access_token_expiration_timestamp_ms: None,
            }
        };

        let current_time_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis() as u64;

        let need_access_token = !cache_exists
            || cache_data.access_token.is_none()
            || cache_data.access_token_expiration_timestamp_ms.is_none()
            || cache_data.access_token_expiration_timestamp_ms.unwrap() < current_time_ms;

        if need_access_token {
            info!("Access token expired or not found, retrieving new token");
            self.get_token().await?;
        } else {
            debug!(
                "Using cached access token (valid until {})",
                cache_data.access_token_expiration_timestamp_ms.unwrap_or(0)
            );
        }

        Ok(())
    }

    /// Retrieves the lyrics of a track from Spotify
    pub async fn get_lyrics(&self, track_id: &str) -> Result<String> {
        // Try up to 2 times in case token needs to be refreshed
        for attempt in 1..=2 {
            self.check_tokens_expire().await?;

            let cache_data = self.load_cache_file()?;
            let token = cache_data
                .access_token
                .ok_or_else(|| SpotifyException::new("Access token not found"))?;

            let formatted_url = format!(
                "{}{}?format=json&market=from_token",
                self.lyrics_url, track_id
            );

            debug!(
                "Requesting lyrics for track {} (attempt {})",
                track_id, attempt
            );

            let response = self
                .client
                .get(&formatted_url)
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (X11; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0",
                )
                .header("referer", "https://open.spotify.com/")
                .header("origin", "https://open.spotify.com/")
                .header("accept", "application/json")
                .header("app-platform", "WebPlayer")
                .header("authorization", format!("Bearer {}", token))
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let result = response.text().await?;
                return Ok(result);
            } else if status.as_u16() == 401 && attempt == 1 {
                // If we get a 401 on the first attempt, force token refresh
                error!("Received 401 Unauthorized, forcing token refresh");

                // Delete the token file to force a complete refresh
                if self.cache_file.exists() {
                    if let Err(e) = std::fs::remove_file(&self.cache_file) {
                        error!("Failed to remove token cache file: {}", e);
                    } else {
                        debug!("Removed token cache file to force refresh");
                    }
                }

                // Continue to the next attempt
                continue;
            } else {
                if status.as_u16() == 404 {
                    return Err(SpotifyException::NotFound);
                }
                Self::check_status(&response, "Lyrics request")?;
            }
        }

        Err(SpotifyException::ApiError(
            "Failed to retrieve lyrics after token refresh".to_string(),
        ))
    }

    /// Extract track ID from a Spotify URL
    pub fn extract_track_id(url: &str) -> Option<String> {
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() > 4 && parts[3] == "track" {
            let track_with_params: Vec<&str> = parts[4].split('?').collect();
            return Some(track_with_params[0].to_string());
        }
        None
    }

    /// Get lyrics in the specified format (id3 or lrc)
    pub async fn get_formatted_lyrics(
        &self,
        track_id: &str,
        format: &str,
    ) -> Result<serde_json::Value> {
        let raw_lyrics = self.get_lyrics(track_id).await?;

        // Parse the JSON response
        let lyrics_data: serde_json::Value = serde_json::from_str(&raw_lyrics)?;

        // Check if lyrics exist
        if !lyrics_data
            .get("lyrics")
            .is_some_and(|lyrics| lyrics["lines"].is_array())
        {
            return Err(SpotifyException::new(
                "Spotify returned an invalid lyrics response",
            ));
        }

        // Determine sync type
        let sync_type = lyrics_data["lyrics"]["syncType"]
            .as_str()
            .unwrap_or("UNSYNCED");

        // Format the lyrics based on the requested format
        if format == "raw" {
            let raw: String = lyrics_data["lyrics"]["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| format!("{}\n", line["words"].as_str().unwrap_or("")))
                .collect();
            Ok(serde_json::json!({"error": false, "syncType": sync_type, "lines": raw}))
        } else if format == "srt" {
            let lines: Vec<_> = lyrics_data["lyrics"]["lines"].as_array().unwrap().windows(2).enumerate()
                .map(|(index, pair)| serde_json::json!({
                    "index": index + 1,
                    "startTime": self.format_srt(pair[0]["startTimeMs"].as_str().unwrap_or("0").parse().unwrap_or(0)),
                    "endTime": self.format_srt(pair[1]["startTimeMs"].as_str().unwrap_or("0").parse().unwrap_or(0)),
                    "words": pair[0]["words"].as_str().unwrap_or("")
                })).collect();
            Ok(serde_json::json!({"error": false, "syncType": sync_type, "lines": lines}))
        } else if format == "lrc" {
            let mut lines = Vec::new();

            if let Some(lyrics_lines) = lyrics_data["lyrics"]["lines"].as_array() {
                for line in lyrics_lines {
                    let start_time_ms = line["startTimeMs"].as_str().unwrap_or("0").to_string();
                    let time_tag = self.format_ms(start_time_ms.parse::<u64>().unwrap_or(0));

                    let lrc_line = LrcLine {
                        time_tag,
                        words: line["words"].as_str().unwrap_or("").to_string(),
                    };

                    lines.push(lrc_line);
                }
            }

            let response = LrcResponse {
                error: false,
                sync_type: sync_type.to_string(),
                lines,
            };

            Ok(serde_json::to_value(response)?)
        } else {
            Ok(serde_json::json!({
                "error": false, "syncType": sync_type, "lines": lyrics_data["lyrics"]["lines"]
            }))
        }
    }

    /// Helper function for getLrcLyrics to change milliseconds to [mm:ss.xx]
    fn format_ms(&self, milliseconds: u64) -> String {
        let total_seconds = milliseconds / 1000;
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        let centiseconds = (milliseconds % 1000) / 10;

        format!("{:02}:{:02}.{:02}", minutes, seconds, centiseconds)
    }

    /// Helper function to format milliseconds to SRT time format (hh:mm:ss,ms)
    fn format_srt(&self, milliseconds: u64) -> String {
        let hours = milliseconds / 3600000;
        let minutes = (milliseconds % 3600000) / 60000;
        let seconds = (milliseconds % 60000) / 1000;
        let ms = milliseconds % 1000;

        format!("{:02}:{:02}:{:02},{:03}", hours, minutes, seconds, ms)
    }
}

#[cfg(test)]
mod tests;
