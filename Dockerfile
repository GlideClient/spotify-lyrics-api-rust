# Build stage
FROM rust:1.83-slim as builder

WORKDIR /usr/src/spotifylyricsapi
COPY . .

# Install dependencies for building
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install SSL certificates and other required dependencies
RUN apt-get update && \
    apt-get install -y ca-certificates libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/spotifylyricsapi/target/release/spotifylyricsapi /app/
COPY --from=builder /usr/src/spotifylyricsapi/config.toml.example /app/config.toml.example

# Create a non-root user to run the application
RUN groupadd -r spotify && useradd -r -g spotify spotify
RUN chown -R spotify:spotify /app
USER spotify

# Expose the default port
EXPOSE 8080

# Command to run
CMD ["./spotifylyricsapi"]