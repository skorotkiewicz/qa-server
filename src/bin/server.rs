use std::{fs, net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    extract::{ConnectInfo, Extension, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use clap::Parser;
use rand::Rng;
use rand::distributions::Alphanumeric;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tower_governor::GovernorError;
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_http::{compression::CompressionLayer, cors::CorsLayer};

use qa_rs::redis_cache::{self, CacheState};

// -- Error handling

/// Typed application errors that convert into HTTP responses.
enum AppError {
    Unauthorized(&'static str),
    Forbidden(&'static str),
    NotFound(&'static str),
    Conflict(&'static str),
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg).into_response(),
            Self::Forbidden(msg) => (StatusCode::FORBIDDEN, msg).into_response(),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg).into_response(),
            Self::Conflict(msg) => (StatusCode::CONFLICT, msg).into_response(),
            // TODO: in production, avoid exposing internal error details to clients
            Self::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}

// -- Config

#[derive(Debug, Deserialize)]
struct ServerConfig {
    bind: Option<String>,
    database_url: String,
    redis_url: Option<String>,
    db_pool_size: Option<u32>,
}

impl ServerConfig {
    fn load(path: &str) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut cfg: ServerConfig = serde_yaml::from_str(&text)?;

        if let Ok(ip) = std::env::var("QA_SERVER_IP") {
            let port = std::env::var("QA_SERVER_PORT")
                .ok()
                .unwrap_or_else(|| "7879".to_string());
            cfg.bind = Some(format!("{}:{}", ip, port));
        } else if let Ok(port) = std::env::var("QA_SERVER_PORT") {
            let ip = cfg
                .bind
                .as_ref()
                .and_then(|b| b.split(':').next())
                .unwrap_or("0.0.0.0")
                .to_string();
            cfg.bind = Some(format!("{}:{}", ip, port));
        }

        if let Ok(db_url) = std::env::var("DATABASE_URL") {
            cfg.database_url = db_url;
        }
        if let Ok(redis_url) = std::env::var("REDIS_URL") {
            cfg.redis_url = Some(redis_url);
        }

        Ok(cfg)
    }
}

// -- Database

type DbPool = sqlx::SqlitePool;

// -- Models

#[derive(Serialize, FromRow)]
struct User {
    id: i64,
    username: String,
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    api_key: String,
    created_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
struct Question {
    id: i64,
    user_id: i64,
    #[sqlx(rename = "username")]
    author: String,
    title: String,
    content: String,
    created_at: DateTime<Utc>,
    solved: bool,
    solved_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    starred: bool,
    #[sqlx(default)]
    views: i64,
    #[sqlx(default)]
    stars: i64,
}

#[derive(Serialize, FromRow)]
struct Answer {
    id: i64,
    question_id: i64,
    user_id: i64,
    #[sqlx(rename = "username")]
    author: String,
    content: String,
    created_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
struct QuestionSummary {
    id: i64,
    title: String,
    #[sqlx(rename = "username")]
    author: String,
    created_at: DateTime<Utc>,
    #[sqlx(default)]
    stars: i64,
    #[sqlx(default)]
    views: i64,
}

// -- API request/response models

#[derive(Deserialize)]
struct CreateUserRequest {
    username: String,
}

#[derive(Deserialize)]
struct ChangeApiKeyRequest {
    current_api_key: String,
}

#[derive(Deserialize)]
struct CreateQuestionRequest {
    title: String,
    content: String,
}

#[derive(Deserialize)]
struct CreateAnswerRequest {
    content: String,
}

#[derive(Serialize)]
struct CreateUserResponse {
    id: i64,
    username: String,
    api_key: String,
}

#[derive(Serialize)]
struct ApiKeyResponse {
    api_key: String,
}

#[derive(Serialize)]
struct QuestionWithAnswers {
    #[serde(flatten)]
    question: Question,
    answers: Vec<Answer>,
}

// -- Pagination

#[derive(Deserialize, Default)]
struct Pagination {
    #[serde(default)]
    page: i64,
    #[serde(default)]
    limit: i64,
}

impl Pagination {
    fn effective_limit(&self, default: i64) -> i64 {
        if self.limit > 0 && self.limit <= 100 {
            self.limit
        } else {
            default
        }
    }

    fn offset(&self, default_limit: i64) -> i64 {
        self.page.max(0) * self.effective_limit(default_limit)
    }
}

// -- Helpers

fn generate_api_key() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

fn get_api_key_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
}

async fn get_user_by_api_key(pool: &DbPool, api_key: &str) -> anyhow::Result<Option<User>> {
    let user = sqlx::query_as::<_, User>(
        "SELECT id, username, api_key, created_at FROM users WHERE api_key = $1",
    )
    .bind(api_key)
    .fetch_optional(pool)
    .await?;
    Ok(user)
}

/// Validate the API key from request headers and return the authenticated user.
async fn authenticate(pool: &DbPool, headers: &HeaderMap) -> Result<User, AppError> {
    let api_key =
        get_api_key_from_headers(headers).ok_or(AppError::Unauthorized("missing api key"))?;
    get_user_by_api_key(pool, api_key)
        .await?
        .ok_or(AppError::Unauthorized("invalid api key"))
}

/// Verify that the given question exists and belongs to the specified user.
async fn verify_question_ownership(
    pool: &DbPool,
    question_id: i64,
    user_id: i64,
    forbidden_msg: &'static str,
) -> Result<(), AppError> {
    let owner_id: Option<i64> = sqlx::query_scalar("SELECT user_id FROM questions WHERE id = $1")
        .bind(question_id)
        .fetch_optional(pool)
        .await?;

    match owner_id {
        Some(id) if id == user_id => Ok(()),
        Some(_) => Err(AppError::Forbidden(forbidden_msg)),
        None => Err(AppError::NotFound("question not found")),
    }
}

/// Verify that the given answer exists, belongs to the specified question and user.
async fn verify_answer_ownership(
    pool: &DbPool,
    question_id: i64,
    answer_id: i64,
    user_id: i64,
) -> Result<(), AppError> {
    let owner_id: Option<i64> =
        sqlx::query_scalar("SELECT user_id FROM answers WHERE id = $1 AND question_id = $2")
            .bind(answer_id)
            .bind(question_id)
            .fetch_optional(pool)
            .await?;

    match owner_id {
        Some(id) if id == user_id => Ok(()),
        Some(_) => Err(AppError::Forbidden("can only delete your own answers")),
        None => Err(AppError::NotFound("answer not found")),
    }
}

/// Shared SQL fragment for question summary queries.
const QUESTION_SUMMARY_SELECT: &str = r#"
    SELECT
        q.id,
        q.title,
        u.username,
        q.created_at,
        (SELECT COUNT(*) FROM stars s WHERE s.question_id = q.id) as stars,
        q.views
    FROM questions q
    JOIN users u ON q.user_id = u.id"#;

// -- Handlers

/// POST /register
async fn create_account(
    State(pool): State<DbPool>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Response, AppError> {
    let api_key = generate_api_key();

    let result = sqlx::query("INSERT INTO users (username, api_key) VALUES ($1, $2)")
        .bind(&req.username)
        .bind(&api_key)
        .execute(&pool)
        .await;

    match result {
        Ok(res) => {
            let resp = CreateUserResponse {
                id: res.last_insert_rowid(),
                username: req.username,
                api_key: api_key.clone(),
            };
            Ok((
                StatusCode::CREATED,
                [("x-api-key", api_key.as_str())],
                Json(resp),
            )
                .into_response())
        }
        Err(e) if e.to_string().contains("UNIQUE") => {
            Err(AppError::Conflict("username already exists"))
        }
        Err(e) => Err(e.into()),
    }
}

/// POST /change-api-key
async fn change_api_key(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Json(req): Json<ChangeApiKeyRequest>,
) -> Result<Response, AppError> {
    let user = authenticate(&pool, &headers).await?;

    if user.api_key != req.current_api_key {
        return Err(AppError::Unauthorized("invalid current api key"));
    }

    let new_api_key = generate_api_key();
    sqlx::query("UPDATE users SET api_key = $1 WHERE id = $2")
        .bind(&new_api_key)
        .bind(user.id)
        .execute(&pool)
        .await?;

    let resp = ApiKeyResponse {
        api_key: new_api_key.clone(),
    };
    Ok((
        StatusCode::OK,
        [("x-api-key", new_api_key.as_str())],
        Json(resp),
    )
        .into_response())
}

/// POST /questions
async fn create_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Json(req): Json<CreateQuestionRequest>,
) -> Result<Response, AppError> {
    let user = authenticate(&pool, &headers).await?;

    let result = sqlx::query("INSERT INTO questions (user_id, title, content) VALUES ($1, $2, $3)")
        .bind(user.id)
        .bind(&req.title)
        .bind(&req.content)
        .execute(&pool)
        .await?;

    let question_id = result.last_insert_rowid();
    Ok((StatusCode::CREATED, question_id.to_string()).into_response())
}

/// GET /questions/unsolved
async fn list_unsolved(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Query(pagination): Query<Pagination>,
) -> Result<Json<Vec<QuestionSummary>>, AppError> {
    let _user = authenticate(&pool, &headers).await?;

    let limit = pagination.effective_limit(20);
    let offset = pagination.offset(20);

    let questions = sqlx::query_as::<_, QuestionSummary>(&format!(
        "{QUESTION_SUMMARY_SELECT}\n\
         WHERE q.solved = FALSE\n\
         ORDER BY q.created_at DESC\n\
         LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .fetch_all(&pool)
    .await?;

    Ok(Json(questions))
}

/// GET /questions/starred
async fn list_starred(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Query(pagination): Query<Pagination>,
) -> Result<Json<Vec<QuestionSummary>>, AppError> {
    let user = authenticate(&pool, &headers).await?;

    let limit = pagination.effective_limit(20);
    let offset = pagination.offset(20);

    let questions = sqlx::query_as::<_, QuestionSummary>(&format!(
        "{QUESTION_SUMMARY_SELECT}\n\
         JOIN stars st ON st.question_id = q.id\n\
         WHERE st.user_id = $1\n\
         ORDER BY q.created_at DESC\n\
         LIMIT $2 OFFSET $3"
    ))
    .bind(user.id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&pool)
    .await?;

    Ok(Json(questions))
}

/// POST /questions/:id/answers
async fn create_answer(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
    Json(req): Json<CreateAnswerRequest>,
) -> Result<Response, AppError> {
    let user = authenticate(&pool, &headers).await?;

    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM questions WHERE id = $1)")
        .bind(question_id)
        .fetch_one(&pool)
        .await
        .unwrap_or(false);

    if !exists {
        return Err(AppError::NotFound("question not found"));
    }

    let result =
        sqlx::query("INSERT INTO answers (question_id, user_id, content) VALUES ($1, $2, $3)")
            .bind(question_id)
            .bind(user.id)
            .bind(&req.content)
            .execute(&pool)
            .await?;

    let answer_id = result.last_insert_rowid();
    Ok((StatusCode::CREATED, answer_id.to_string()).into_response())
}

/// POST /questions/:id/solved
async fn mark_solved(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<StatusCode, AppError> {
    let user = authenticate(&pool, &headers).await?;

    verify_question_ownership(
        &pool,
        question_id,
        user.id,
        "only the asker can mark as solved/unsolved",
    )
    .await?;

    let unsolved = params.get("unsolved").map(|v| v == "true").unwrap_or(false);

    if unsolved {
        sqlx::query("UPDATE questions SET solved = FALSE, solved_at = NULL WHERE id = $1")
            .bind(question_id)
            .execute(&pool)
            .await?;
    } else {
        sqlx::query(
            "UPDATE questions SET solved = TRUE, solved_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(question_id)
        .execute(&pool)
        .await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /questions/:id/star
async fn star_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
) -> Result<StatusCode, AppError> {
    let user = authenticate(&pool, &headers).await?;

    let result = sqlx::query(
        "INSERT INTO stars (user_id, question_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user.id)
    .bind(question_id)
    .execute(&pool)
    .await;

    match result {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) if e.to_string().contains("FOREIGN KEY") => {
            Err(AppError::NotFound("question not found"))
        }
        Err(e) => Err(e.into()),
    }
}

/// DELETE /questions/:id/star
async fn unstar_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
) -> Result<StatusCode, AppError> {
    let user = authenticate(&pool, &headers).await?;

    sqlx::query("DELETE FROM stars WHERE user_id = $1 AND question_id = $2")
        .bind(user.id)
        .bind(question_id)
        .execute(&pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /questions/:id
async fn get_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
) -> Result<Json<QuestionWithAnswers>, AppError> {
    let _user = authenticate(&pool, &headers).await?;

    // Increment view count (best-effort, ignore errors)
    sqlx::query("UPDATE questions SET views = views + 1 WHERE id = $1")
        .bind(question_id)
        .execute(&pool)
        .await
        .ok();

    let question = sqlx::query_as::<_, Question>(
        r#"
        SELECT
            q.id, q.user_id, u.username, q.title, q.content,
            q.created_at, q.solved, q.solved_at, 0 as starred, q.views,
            (SELECT COUNT(*) FROM stars s WHERE s.question_id = q.id) as stars
        FROM questions q
        JOIN users u ON q.user_id = u.id
        WHERE q.id = $1
        "#,
    )
    .bind(question_id)
    .fetch_optional(&pool)
    .await?
    .ok_or(AppError::NotFound("question not found"))?;

    let answers = sqlx::query_as::<_, Answer>(
        r#"
        SELECT a.id, a.question_id, a.user_id, u.username, a.content, a.created_at
        FROM answers a
        JOIN users u ON a.user_id = u.id
        WHERE a.question_id = $1
        ORDER BY a.created_at ASC
        "#,
    )
    .bind(question_id)
    .fetch_all(&pool)
    .await?;

    Ok(Json(QuestionWithAnswers { question, answers }))
}

/// DELETE /questions/:id
async fn delete_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
) -> Result<StatusCode, AppError> {
    let user = authenticate(&pool, &headers).await?;

    verify_question_ownership(
        &pool,
        question_id,
        user.id,
        "can only delete your own questions",
    )
    .await?;

    sqlx::query("DELETE FROM questions WHERE id = $1")
        .bind(question_id)
        .execute(&pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /questions/:id/answers/:answer_id
async fn delete_answer(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path((question_id, answer_id)): Path<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    let user = authenticate(&pool, &headers).await?;

    verify_answer_ownership(&pool, question_id, answer_id, user.id).await?;

    sqlx::query("DELETE FROM answers WHERE id = $1")
        .bind(answer_id)
        .execute(&pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /health
async fn health() -> &'static str {
    "ok"
}

// -- Rate-limiting key extractor

#[derive(Clone)]
struct ApiKeyExtractor;

impl KeyExtractor for ApiKeyExtractor {
    type Key = Arc<str>;

    fn extract<B>(&self, req: &axum::http::Request<B>) -> Result<Self::Key, GovernorError> {
        if let Some(api_key) = req
            .headers()
            .get("authorization")
            .and_then(|h| h.to_str().ok())
        {
            return Ok(Arc::from(api_key));
        }

        if let Some(connect_info) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
            return Ok(Arc::from(format!("ip:{}", connect_info.0.ip())));
        }
        Ok(Arc::from("anonymous"))
    }
}

// -- Main

#[derive(Parser)]
#[command(name = "qa-server", about = "Q&A server", version)]
struct Args {
    /// Path to config file
    #[arg(default_value = "server.yml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cfg = ServerConfig::load(&args.config)?;

    let bind: SocketAddr = cfg.bind.as_deref().unwrap_or("0.0.0.0:7879").parse()?;

    // Ensure database file exists for SQLite
    let db_path = cfg
        .database_url
        .strip_prefix("sqlite://")
        .or_else(|| cfg.database_url.strip_prefix("sqlite:"))
        .unwrap_or(&cfg.database_url);

    if !std::path::Path::new(db_path).exists() {
        std::fs::File::create(db_path)?;
    }

    let pool_size = cfg.db_pool_size.unwrap_or(10);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(pool_size)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .max_lifetime(std::time::Duration::from_secs(3600))
        .idle_timeout(std::time::Duration::from_secs(600))
        .connect(&cfg.database_url)
        .await?;

    // Optimize SQLite for performance
    for pragma in [
        "PRAGMA journal_mode=WAL",
        "PRAGMA synchronous=NORMAL",
        "PRAGMA cache_size=-64000",
        "PRAGMA temp_store=memory",
        "PRAGMA mmap_size=30000000000",
    ] {
        sqlx::query(pragma).execute(&pool).await.ok();
    }

    sqlx::migrate!("./migrations").run(&pool).await?;
    println!("✓ database connected ({db_path})");

    // Connect to Redis if configured
    let redis_conn = match &cfg.redis_url {
        Some(url) => redis_cache::connect(url).await,
        None => {
            println!("⚠ redis not configured (set redis_url in server.yml)");
            None
        }
    };
    let cache_state = CacheState::new(redis_conn);

    // Rate limiting: 30 requests/sec with burst of 60
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(30)
        .burst_size(60)
        .key_extractor(ApiKeyExtractor)
        .finish()
        .unwrap();

    let public_routes = Router::new()
        .route("/health", get(health))
        .route("/register", post(create_account));

    let cached_routes = Router::new()
        .route("/questions/unsolved", get(list_unsolved))
        .route("/questions/starred", get(list_starred))
        .route("/questions/{id}", get(get_question))
        .layer(axum::middleware::from_fn(redis_cache::cache_middleware));

    let auth_routes = Router::new()
        .route("/change-api-key", post(change_api_key))
        .route("/questions", post(create_question))
        .route("/questions/{id}", delete(delete_question))
        .route("/questions/{id}/answers", post(create_answer))
        .route("/questions/{id}/answers/{answer_id}", delete(delete_answer))
        .route("/questions/{id}/solved", post(mark_solved))
        .route(
            "/questions/{id}/star",
            post(star_question).delete(unstar_question),
        );

    let app = Router::new()
        .merge(public_routes)
        .merge(cached_routes)
        .merge(auth_routes)
        .layer(Extension(cache_state))
        .layer(CompressionLayer::new())
        .layer(GovernorLayer::new(governor_conf))
        .layer(CorsLayer::permissive())
        .with_state(pool);

    println!("✓ qa-server listening on {bind}");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
