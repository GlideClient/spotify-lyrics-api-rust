mod spotify;
mod spotifyexception;
mod config;

use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Responder, middleware::Logger};
use spotify::Spotify;
use spotifyexception::SpotifyException;
use std::sync::Mutex;
use log::{info, error};
use serde_json::json;
use config::Config;

// Struct to hold application state
struct AppState {
    spotify: Mutex<Spotify>,
}

// Handler for the main endpoint that processes GET requests with query parameters
async fn get_lyrics(
    query: web::Query<std::collections::HashMap<String, String>>,
    data: web::Data<AppState>
) -> impl Responder {
    // Get the spotify client from state
    let spotify = data.spotify.lock().unwrap();
    
    // Check if trackid or url is provided
    let track_id = if let Some(trackid) = query.get("trackid") {
        trackid.to_string()
    } else if let Some(url) = query.get("url") {
        if let Some(extracted_id) = Spotify::extract_track_id(url) {
            extracted_id
        } else {
            return HttpResponse::BadRequest()
                .json(json!({
                    "error": true,
                    "message": "invalid url parameter!"
                }));
        }
    } else {
        return HttpResponse::BadRequest()
            .json(json!({
                "error": true,
                "message": "url or trackid parameter is required!"
            }));
    };
    
    // Get format parameter with default as "id3"
    let format = query.get("format").unwrap_or(&"id3".to_string()).to_string();
    
    // Only accept "id3" or "lrc" as formats
    if format != "id3" && format != "lrc" {
        return HttpResponse::BadRequest()
            .json(json!({
                "error": true,
                "message": "format parameter must be either 'id3' or 'lrc'!"
            }));
    }
    
    info!("Getting lyrics for track: {}, format: {}", track_id, format);
    
    match spotify.get_formatted_lyrics(&track_id, &format).await {
        Ok(lyrics_json) => {
            HttpResponse::Ok().json(lyrics_json)
        },
        Err(e) => {
            match e {
                SpotifyException::Generic(ref message) if message == "lyrics for this track is not available on spotify!" => {
                    HttpResponse::NotFound()
                        .json(json!({
                            "error": true,
                            "message": "lyrics for this track is not available on spotify!"
                        }))
                },
                _ => {
                    eprintln!("Error fetching lyrics: {}", e);
                    HttpResponse::InternalServerError()
                        .json(json!({
                            "error": true,
                            "message": format!("Failed to fetch lyrics: {}", e)
                        }))
                }
            }
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
