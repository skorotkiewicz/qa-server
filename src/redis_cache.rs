// redis.rs - Redis caching module for QA server

use axum::{
    body::Body,
    extract::Extension,
    http::{Request, Response, StatusCode},
    middleware::Next,
    response::IntoResponse,
};
use redis::AsyncCommands;

/// Cache state wrapper around Redis connection
#[derive(Clone)]
pub struct CacheState {
    redis: Option<redis::aio::MultiplexedConnection>,
}

impl CacheState {
    /// Create new CacheState (redis is optional - works without Redis)
    pub fn new(redis: Option<redis::aio::MultiplexedConnection>) -> Self {
        Self { redis }
    }

    /// Get value from cache
    pub async fn get(&self, key: &str) -> Option<String> {
        if let Some(ref redis) = self.redis {
            let mut conn = redis.clone();
            conn.get(key).await.ok().flatten()
        } else {
            None
        }
    }

    /// Set value in cache with TTL (seconds)
    pub async fn set(&self, key: &str, value: &str, ttl: u64) {
        if let Some(ref redis) = self.redis {
            let mut conn = redis.clone();
            let _: Result<(), _> = conn.set_ex(key, value, ttl).await;
        }
    }
}

/// Generate cache key from request and optional API key
/// Format: qa:cache:{method}:{uri}:user:{api_key} or qa:cache:{method}:{uri}:anon
fn generate_cache_key(req: &Request<Body>, api_key: Option<&str>) -> String {
    let uri = req.uri().to_string();
    let method = req.method().as_str();
    let key_base = format!("{}:{}", method, uri);
    match api_key {
        Some(key) => format!("qa:cache:{}:user:{}", key_base, key),
        None => format!("qa:cache:{}:anon", key_base),
    }
}

/// Axum middleware that caches GET responses in Redis
/// Adds X-Cache: HIT or X-Cache: MISS header to responses
pub async fn cache_middleware(
    Extension(cache): Extension<CacheState>,
    req: Request<Body>,
    next: Next,
) -> Response<Body> {
    // Only cache GET requests
    if req.method() != axum::http::Method::GET {
        return next.run(req).await;
    }

    // Get API key for cache segmentation (different cache per user)
    let api_key = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok());

    let cache_key = generate_cache_key(&req, api_key);

    // Try to get from cache
    if let Some(cached_value) = cache.get(&cache_key).await {
        let body = Body::from(cached_value);
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .header("x-cache", "HIT")
            .body(body)
            .unwrap_or_else(|_| {
                (StatusCode::INTERNAL_SERVER_ERROR, "cache error").into_response()
            });
    }

    // Not in cache, proceed with request
    let response = next.run(req).await;

    // Cache successful responses (200 OK)
    if response.status() == StatusCode::OK {
        let (parts, body) = response.into_parts();

        // Get body bytes
        match axum::body::to_bytes(body, usize::MAX).await {
            Ok(bytes) => {
                let body_str = String::from_utf8_lossy(&bytes);

                // TTL: 60s for /unsolved list, 300s for individual questions
                let ttl = if cache_key.contains("unsolved") {
                    60
                } else {
                    300
                };
                cache.set(&cache_key, &body_str, ttl).await;

                // Rebuild response with X-Cache: MISS header
                let mut response = Response::from_parts(parts, Body::from(bytes));
                response
                    .headers_mut()
                    .insert("x-cache", "MISS".parse().unwrap());
                response
            }
            Err(_) => {
                // Failed to read body - return empty response
                Response::from_parts(parts, Body::empty())
            }
        }
    } else {
        response
    }
}

/// Connect to Redis and return connection (or None if failed/unconfigured)
pub async fn connect(redis_url: &str) -> Option<redis::aio::MultiplexedConnection> {
    match redis::Client::open(redis_url) {
        Ok(client) => match client.get_multiplexed_async_connection().await {
            Ok(conn) => {
                println!("✓ redis connected");
                Some(conn)
            }
            Err(e) => {
                eprintln!("⚠ redis connection failed: {}", e);
                None
            }
        },
        Err(e) => {
            eprintln!("⚠ redis client creation failed: {}", e);
            None
        }
    }
}
