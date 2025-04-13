# Spotify Lyrics API

A Rust-based API server that fetches synchronized lyrics from Spotify and provides them in multiple formats. This is a complete rewrite in Rust of [spotify-lyrics-api](https://github.com/akashrchandran/spotify-lyrics-api).

## Features

- Fetch time-synchronized lyrics from Spotify's internal API
- Support for multiple output formats (ID3, LRC)
- Simple HTTP endpoint for easy integration with other applications
- CORS support for web applications
- Configurable via config file or environment variables
- Automatic token management and caching
- Extract track IDs from full Spotify URLs

## Requirements

- Rust (latest stable version recommended)
- A valid Spotify cookie (SP_DC) from a premium account

## Installation

### From Source

1. Clone the repository:
```sh
git clone https://github.com/yourusername/spotifylyricsapi.git
cd spotifylyricsapi
```

2. Build the project:
```sh
cargo build --release
```

3. The compiled binary will be available at `target/release/spotifylyricsapi`

### Using Docker

You can run the application using Docker in two ways:

#### Using Docker directly

1. Build the Docker image:
```sh
docker build -t spotifylyricsapi .
```

2. Run the container:
```sh
docker run -d -p 8080:8080 -e SP_DC=your_spotify_cookie_value spotifylyricsapi
```

#### Using Docker Compose

1. Set your SP_DC environment variable:
```sh
export SP_DC=your_spotify_cookie_value
```

2. Run with docker-compose:
```sh
docker-compose up -d
```

This will build the image if it doesn't exist and start the container.

## Configuration

Create a `config.toml` file in one of these locations:
- Current directory (`./config.toml`)
- User's config directory (`~/.config/spotifylyricsapi/config.toml`) 
- System-wide (`/etc/spotifylyricsapi/config.toml`)

```toml
# Spotify Lyrics API Configuration

# Your Spotify cookie value (required)
# This is the value of the SP_DC cookie from your Spotify web session
sp_dc = "YOUR_SP_DC_COOKIE_VALUE_HERE"

# Server port (optional, defaults to 8080 if not specified)
# port = 8080
```

Alternatively, you can set these environment variables:
- `SP_DC`: Your Spotify cookie value
- `PORT`: The port to run the server on (defaults to 8080)

### How to get your Spotify Cookie (SP_DC)

1. Log in to [Spotify Web Player](https://open.spotify.com/)
2. Open your browser's developer tools (F12 or right-click > Inspect)
3. Go to the Application/Storage tab
4. Find Cookies > https://open.spotify.com
5. Copy the value of the `sp_dc` cookie

## Usage

### Starting the Server

```sh
./spotifylyricsapi
```

The server will start on port 8080 by default (or the configured port).

### API Endpoints

#### GET /

Fetches lyrics for a Spotify track.

**Query Parameters:**
- `trackid`: The Spotify track ID (Required if URL is not provided)
- `url`: A Spotify track URL (Required if trackid is not provided)
- `format`: Output format - either `id3` or `lrc` (Default: `id3`)

**Examples:**
- Using track ID: `http://localhost:8080/?trackid=4cOdK2wGLETKBW3PvgPWqT`
- Using URL: `http://localhost:8080/?url=https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT`
- Using LRC format: `http://localhost:8080/?trackid=4cOdK2wGLETKBW3PvgPWqT&format=lrc`

**Response Format (ID3):**
```json
{
  "error": false,
  "syncType": "LINE_SYNCED",
  "lines": [
    {
      "startTimeMs": "1230",
      "words": "Look at the stars",
      "syllables": [],
      "endTimeMs": "0"
    }
  ]
}
```

**Response Format (LRC):**
```json
{
  "error": false,
  "syncType": "LINE_SYNCED",
  "lines": [
    {
      "timeTag": "00:01.23",
      "words": "Look at the stars"
    }
  ]
}
```

### Error Responses

**400 Bad Request:**
```json
{
  "error": true,
  "message": "url or trackid parameter is required!"
}
```

**404 Not Found:**
```json
{
  "error": true,
  "message": "lyrics for this track is not available on spotify!"
}
```

## Integration Examples

### cURL
```sh
curl "http://localhost:8080/?trackid=4cOdK2wGLETKBW3PvgPWqT"
```

### JavaScript
```javascript
fetch('http://localhost:8080/?trackid=4cOdK2wGLETKBW3PvgPWqT')
  .then(response => response.json())
  .then(data => console.log(data));
```

## License

This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

## Disclaimer

This project is not affiliated with, maintained, authorized, endorsed, or sponsored by Spotify. This is an independent project that uses Spotify's internal API, which may change without notice.