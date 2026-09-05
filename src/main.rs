mod config;
mod spotify;
mod spotifyexception;

use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use config::Config;
use log::{error, info};
use serde_json::json;
use spotify::Spotify;
use spotifyexception::SpotifyException;
use tokio::sync::Mutex;

// Struct to hold application state
struct AppState {
    spotify: Mutex<Spotify>,
}

// Handler for the main endpoint that processes GET requests with query parameters
async fn get_lyrics(
    query: web::Query<std::collections::HashMap<String, String>>,
    data: web::Data<AppState>,
) -> impl Responder {
    // Check if trackid or url is provided
    let track_id = if let Some(trackid) = query.get("trackid") {
        trackid.to_string()
    } else if let Some(url) = query.get("url") {
        if let Some(extracted_id) = Spotify::extract_track_id(url) {
            extracted_id
        } else {
            return HttpResponse::BadRequest().json(json!({
                "error": true,
                "message": "invalid url parameter!"
            }));
        }
    } else {
        return HttpResponse::BadRequest().json(json!({
            "error": true,
            "message": "url or trackid parameter is required!"
        }));
    };

    // Get format parameter with default as "id3"
    let format = query
        .get("format")
        .unwrap_or(&"id3".to_string())
        .to_string();

    // Match the output formats supported by upstream.
    if !matches!(format.as_str(), "id3" | "lrc" | "srt" | "raw") {
        return HttpResponse::BadRequest().json(json!({
            "error": true,
            "message": "format parameter must be 'id3', 'lrc', 'srt' or 'raw'!"
        }));
    }

    info!("Getting lyrics for track: {}, format: {}", track_id, format);

    let spotify = data.spotify.lock().await;
    match spotify.get_formatted_lyrics(&track_id, &format).await {
        Ok(lyrics_json) => HttpResponse::Ok().json(lyrics_json),
        Err(e) => error_response(e),
    }
}

fn error_response(e: SpotifyException) -> HttpResponse {
    match e {
        SpotifyException::RateLimited { ref retry_after } => {
            let mut response = HttpResponse::TooManyRequests();
            if let Some(value) = retry_after {
                response.insert_header(("Retry-After", value.as_str()));
            }
            response.json(json!({"error": true, "message": e.to_string()}))
        }
        SpotifyException::NotFound => HttpResponse::NotFound().json(json!({
            "error": true,
            "message": "lyrics for this track is not available on spotify!"
        })),
        _ => {
            eprintln!("Error fetching lyrics: {}", e);
            HttpResponse::InternalServerError().json(json!({
                "error": true,
                "message": format!("Failed to fetch lyrics: {}", e)
            }))
        }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize the logger
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    // Load configuration from file or environment variables
    let config = Config::load();

    if !config.is_valid() {
        error!("No SP_DC token found. Please set it in your config file or environment variable.");
        error!("Create a config file at one of these locations:");
        error!("  - ./config.toml");
        error!("  - ~/.config/spotifylyricsapi/config.toml");
        error!("  - /etc/spotifylyricsapi/config.toml");
        error!("With the content: sp_dc = \"your_spotify_cookie_value\"");
        error!("Or set the SP_DC environment variable.");
        std::process::exit(1);
    }

    info!("Starting server at http://127.0.0.1:{}", config.port);

    // Create a new Spotify client
    let spotify = Spotify::new(config.sp_dc);

    // Create application state
    let app_state = web::Data::new(AppState {
        spotify: Mutex::new(spotify),
    });

    // Start the HTTP server
    HttpServer::new(move || {
        // Configure CORS
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            .app_data(app_state.clone())
            .route("/", web::get().to(get_lyrics))
    })
    .bind(("0.0.0.0", config.port))?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_rate_limit_and_not_found_statuses() {
        let limited = error_response(SpotifyException::RateLimited {
            retry_after: Some("17".into()),
        });
        assert_eq!(
            limited.status(),
            actix_web::http::StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(limited.headers().get("retry-after").unwrap(), "17");
        let limited = error_response(SpotifyException::RateLimited { retry_after: None });
        assert!(!limited.headers().contains_key("retry-after"));
        assert_eq!(
            error_response(SpotifyException::NotFound).status(),
            actix_web::http::StatusCode::NOT_FOUND
        );
    }
}
