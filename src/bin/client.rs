use std::{
    fs,
    io::{self, Read},
    path::PathBuf,
};

use anyhow::Context;
use clap::{CommandFactory, Parser, Subcommand};
use serde::{Deserialize, Serialize};

// -- Config

#[derive(Debug, Deserialize, Serialize)]
struct ClientConfig {
    endpoint: String,
    api_key: Option<String>,
    username: Option<String>,
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/qa/config.yml")
}

fn load_config() -> anyhow::Result<ClientConfig> {
    let path = config_path();
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "Config not found at {}. Run `qa create-account` first.",
            path.display()
        )
    })?;
    Ok(serde_yaml::from_str(&text)?)
}

fn save_config(cfg: &ClientConfig) -> anyhow::Result<()> {
    let path = config_path();
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, serde_yaml::to_string(cfg)?)?;
    Ok(())
}

// -- HTTP helpers

fn client_with_api_key(api_key: &str) -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("authorization", api_key.parse().unwrap());
            h
        })
        .build()
        .unwrap()
}

fn get_api_key_from_response(resp: &reqwest::blocking::Response) -> Option<String> {
    resp.headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

// -- Models

#[derive(Deserialize)]
#[allow(dead_code)]
struct CreateAccountResponse {
    id: i64,
    username: String,
    api_key: String,
}

#[derive(Deserialize)]
struct QuestionSummary {
    id: i64,
    title: String,
    #[allow(dead_code)]
    author: String,
    created_at: String,
    stars: i64,
    views: i64,
}

#[derive(Deserialize)]
struct Question {
    id: i64,
    #[allow(dead_code)]
    user_id: i64,
    author: String,
    title: String,
    content: String,
    #[allow(dead_code)]
    created_at: String,
    solved: bool,
    #[allow(dead_code)]
    solved_at: Option<String>,
    #[allow(dead_code)]
    starred: bool,
    #[allow(dead_code)]
    views: i64,
    stars: i64,
}

#[derive(Deserialize)]
struct Answer {
    id: i64,
    #[allow(dead_code)]
    question_id: i64,
    #[allow(dead_code)]
    user_id: i64,
    author: String,
    content: String,
    created_at: String,
}

#[derive(Deserialize)]
struct QuestionWithAnswers {
    #[serde(flatten)]
    question: Question,
    answers: Vec<Answer>,
}

// -- App (shared state for CLI commands)

/// Holds the loaded config and provides convenience methods.
struct App {
    config: ClientConfig,
}

impl App {
    fn load() -> anyhow::Result<Self> {
        Ok(Self {
            config: load_config()?,
        })
    }

    fn api_key(&self) -> anyhow::Result<&str> {
        self.config
            .api_key
            .as_deref()
            .context("No API key found. Run `qa create-account` first.")
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.endpoint, path)
    }

    fn client(&self) -> anyhow::Result<reqwest::blocking::Client> {
        Ok(client_with_api_key(self.api_key()?))
    }
}

// -- Command implementations

fn cmd_create_account() -> anyhow::Result<()> {
    let endpoint = prompt_endpoint()?;
    let username = prompt("Username: ")?;

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("{}/register", endpoint))
        .json(&serde_json::json!({ "username": username }))
        .send()?;

    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    let api_key =
        get_api_key_from_response(&resp).context("Server did not return x-api-key header")?;

    let body_text = resp.text()?;
    let body: CreateAccountResponse =
        serde_json::from_str(&body_text).unwrap_or(CreateAccountResponse {
            id: 0,
            username: username.clone(),
            api_key: api_key.clone(),
        });

    save_config(&ClientConfig {
        endpoint,
        api_key: Some(api_key.clone()),
        username: Some(username.clone()),
    })?;

    println!("✓ Account created: {}", body.username);
    println!("✓ API key saved to {}", config_path().display());
    println!("  API Key: {}", api_key);
    Ok(())
}

fn cmd_change_api_key() -> anyhow::Result<()> {
    let mut cfg = load_config()?;
    let current_api_key = cfg
        .api_key
        .as_ref()
        .context("No API key found. Run `qa create-account` first.")?;

    let client = client_with_api_key(current_api_key);
    let resp = client
        .post(format!("{}/change-api-key", cfg.endpoint))
        .json(&serde_json::json!({ "current_api_key": current_api_key }))
        .send()?;

    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    let new_api_key =
        get_api_key_from_response(&resp).context("Server did not return x-api-key header")?;

    cfg.api_key = Some(new_api_key.clone());
    save_config(&cfg)?;

    println!("✓ API key changed successfully");
    println!("  New API Key: {}", new_api_key);
    Ok(())
}

fn cmd_ask() -> anyhow::Result<()> {
    let app = App::load()?;

    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    let title = input
        .lines()
        .next()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .context("No input provided. Expected title on first line, then content.")?
        .to_string();

    let content = input.trim().to_string();

    let resp = app
        .client()?
        .post(app.url("/questions"))
        .json(&serde_json::json!({ "title": title, "content": content }))
        .send()?;

    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    let question_id: i64 = resp
        .text()?
        .parse()
        .context("Failed to parse question ID")?;
    println!("✓ Question created: #{}", question_id);
    Ok(())
}

fn cmd_unsolved(page: i64, limit: i64) -> anyhow::Result<()> {
    let app = App::load()?;
    let url = app.url(&format!(
        "/questions/unsolved?page={}&limit={}",
        page, limit
    ));

    let resp = app.client()?.get(&url).send()?;
    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    let questions: Vec<QuestionSummary> = resp.json()?;
    if questions.is_empty() {
        println!("No unsolved questions.");
    } else {
        print_question_list(&questions);
        println!("  => show: `qa get <id>` | answer: `echo \"your answer\" | qa answer <id>`");
        println!("  => next page: `qa unsolved --page {}`", page + 1);
    }
    Ok(())
}

fn cmd_starred(page: i64, limit: i64) -> anyhow::Result<()> {
    let app = App::load()?;
    let url = app.url(&format!("/questions/starred?page={}&limit={}", page, limit));

    let resp = app.client()?.get(&url).send()?;
    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    let questions: Vec<QuestionSummary> = resp.json()?;
    if questions.is_empty() {
        println!("No starred questions.");
    } else {
        print_question_list(&questions);
        println!("  => show: `qa get <id>` | answer: `echo \"your answer\" | qa answer <id>`");
        println!("  => next page: `qa starred --page {}`", page + 1);
    }
    Ok(())
}

fn cmd_get(id: i64) -> anyhow::Result<()> {
    let app = App::load()?;
    let resp = app
        .client()?
        .get(&app.url(&format!("/questions/{}", id)))
        .send()?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("Question #{} not found", id);
    }
    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    let qa: QuestionWithAnswers = resp.json()?;
    let q = &qa.question;

    let status = if q.solved { "[SOLVED]" } else { "[OPEN]" };
    let stars = if q.stars > 0 {
        format!(", stars:{}", q.stars)
    } else {
        String::new()
    };
    let time_ago = format_time_ago(&q.created_at);

    println!(
        "{} [question: {}] {} at {} by {} [views:{}{}]",
        status, q.id, q.title, time_ago, q.author, q.views, stars
    );
    println!("\n{}", q.content);
    println!("\n--- {} Answers ---\n", qa.answers.len());

    for ans in &qa.answers {
        let time_ago = format_time_ago(&ans.created_at);
        println!("[answer: {}] at {} by {}", ans.id, time_ago, ans.author);
        println!("{}", ans.content);
        println!("--------");
    }
    Ok(())
}

fn cmd_answer(id: i64) -> anyhow::Result<()> {
    let app = App::load()?;

    let mut content = String::new();
    io::stdin().read_to_string(&mut content)?;
    let content = content.trim();

    if content.is_empty() {
        anyhow::bail!("Answer content cannot be empty");
    }

    let resp = app
        .client()?
        .post(app.url(&format!("/questions/{}/answers", id)))
        .json(&serde_json::json!({ "content": content }))
        .send()?;

    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    let answer_id: i64 = resp.text()?.parse().context("Failed to parse answer ID")?;
    println!("✓ Answer created: #{} for question #{}", answer_id, id);
    Ok(())
}

fn cmd_solve(id: i64, unsolved: bool) -> anyhow::Result<()> {
    let app = App::load()?;
    let url = app.url(&format!("/questions/{}/solved?unsolved={}", id, unsolved));

    let resp = app.client()?.post(&url).send()?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("Only the person who asked the question can mark it as solved/unsolved");
    }
    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    println!(
        "✓ Question #{} marked as {}",
        id,
        if unsolved { "unsolved" } else { "solved" }
    );
    Ok(())
}

fn cmd_star(id: i64) -> anyhow::Result<()> {
    let app = App::load()?;
    let resp = app
        .client()?
        .post(app.url(&format!("/questions/{}/star", id)))
        .send()?;

    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    println!("✓ Starred question #{}", id);
    Ok(())
}

fn cmd_unstar(id: i64) -> anyhow::Result<()> {
    let app = App::load()?;
    let resp = app
        .client()?
        .delete(app.url(&format!("/questions/{}/star", id)))
        .send()?;

    if !resp.status().is_success() {
        anyhow::bail!("Server error: {}", resp.text()?);
    }

    println!("✓ Unstarred question #{}", id);
    Ok(())
}

fn cmd_rm_question(id: i64) -> anyhow::Result<()> {
    let app = App::load()?;
    let resp = app
        .client()?
        .delete(app.url(&format!("/questions/{}", id)))
        .send()?;

    match resp.status() {
        reqwest::StatusCode::FORBIDDEN => anyhow::bail!("You can only delete your own questions"),
        reqwest::StatusCode::NOT_FOUND => anyhow::bail!("Question #{} not found", id),
        s if !s.is_success() => anyhow::bail!("Server error: {}", resp.text()?),
        _ => {}
    }

    println!("✓ Deleted question #{} with all its answers", id);
    Ok(())
}

fn cmd_rm_answer(question_id: i64, answer_id: i64) -> anyhow::Result<()> {
    let app = App::load()?;
    let resp = app
        .client()?
        .delete(app.url(&format!("/questions/{}/answers/{}", question_id, answer_id)))
        .send()?;

    match resp.status() {
        reqwest::StatusCode::FORBIDDEN => anyhow::bail!("You can only delete your own answers"),
        reqwest::StatusCode::NOT_FOUND => {
            anyhow::bail!(
                "Answer #{} not found for question #{}",
                answer_id,
                question_id
            )
        }
        s if !s.is_success() => anyhow::bail!("Server error: {}", resp.text()?),
        _ => {}
    }

    println!(
        "✓ Deleted answer #{} from question #{}",
        answer_id, question_id
    );
    Ok(())
}

// -- Display helpers

fn print_question_list(questions: &[QuestionSummary]) {
    for q in questions {
        let stars = if q.stars > 0 {
            format!(", stars:{}", q.stars)
        } else {
            String::new()
        };
        let time_ago = format_time_ago(&q.created_at);
        println!(
            "[question: {}] {} at {} by {} [views:{}{}]",
            q.id, q.title, time_ago, q.author, q.views, stars
        );
    }
}

fn format_time_ago(date_str: &str) -> String {
    let Ok(created) = chrono::DateTime::parse_from_rfc3339(date_str) else {
        return "unknown".to_string();
    };

    let duration = chrono::Utc::now().signed_duration_since(created.with_timezone(&chrono::Utc));
    let seconds = duration.num_seconds();
    let minutes = duration.num_minutes();
    let hours = duration.num_hours();
    let days = duration.num_days();

    if seconds < 60 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{}m ago", minutes)
    } else if hours < 24 {
        format!("{}h ago", hours)
    } else if days < 7 {
        format!("{}d ago", days)
    } else if days < 30 {
        format!("{}w ago", days / 7)
    } else if days < 365 {
        format!("{}mo ago", days / 30)
    } else {
        format!("{}y ago", days / 365)
    }
}

// -- Interactive prompts

fn prompt(message: &str) -> anyhow::Result<String> {
    print!("{}", message);
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn prompt_endpoint() -> anyhow::Result<String> {
    let default = load_config()
        .map(|c| c.endpoint)
        .unwrap_or_else(|_| "http://localhost:7878".to_string());

    let endpoint = prompt(&format!("Server endpoint [{}]: ", default))?;
    Ok(if endpoint.is_empty() {
        default
    } else {
        endpoint
    })
}

// -- CLI

#[derive(Parser)]
#[command(name = "qa", about = "QA system CLI", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new account and save API key
    CreateAccount,
    /// Change your API key (requires current API key)
    ChangeApiKey,
    /// Ask a question (reads from stdin, expects title on first line, then content)
    Ask,
    /// List unsolved questions (paginated)
    Unsolved {
        /// Page number (0-indexed)
        #[arg(short, long, default_value = "0")]
        page: i64,
        /// Page size (1-100)
        #[arg(short, long, default_value = "20")]
        limit: i64,
    },
    /// List questions you have starred (paginated)
    Starred {
        /// Page number (0-indexed)
        #[arg(short, long, default_value = "0")]
        page: i64,
        /// Page size (1-100)
        #[arg(short, long, default_value = "20")]
        limit: i64,
    },
    /// Get a question with all its answers
    Get { id: i64 },
    /// Answer a question (reads from stdin)
    Answer { id: i64 },
    /// Mark a question as solved or unsolved (only the asker can do this)
    Solve {
        id: i64,
        /// Mark as unsolved instead of solved
        #[arg(long)]
        unsolved: bool,
    },
    /// Star a question
    Star { id: i64 },
    /// Unstar a question
    Unstar { id: i64 },
    /// Remove your own question (with all its answers)
    RmQuestion { id: i64 },
    /// Remove your own answer
    RmAnswer { question_id: i64, answer_id: i64 },
    /// Generate shell completion (add `eval "$(qa complete zsh)"` to .zshrc)
    Complete { shell: clap_complete::Shell },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::CreateAccount => cmd_create_account(),
        Cmd::ChangeApiKey => cmd_change_api_key(),
        Cmd::Ask => cmd_ask(),
        Cmd::Unsolved { page, limit } => cmd_unsolved(page, limit),
        Cmd::Starred { page, limit } => cmd_starred(page, limit),
        Cmd::Get { id } => cmd_get(id),
        Cmd::Answer { id } => cmd_answer(id),
        Cmd::Solve { id, unsolved } => cmd_solve(id, unsolved),
        Cmd::Star { id } => cmd_star(id),
        Cmd::Unstar { id } => cmd_unstar(id),
        Cmd::RmQuestion { id } => cmd_rm_question(id),
        Cmd::RmAnswer {
            question_id,
            answer_id,
        } => cmd_rm_answer(question_id, answer_id),
        Cmd::Complete { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "qa", &mut std::io::stdout());
            Ok(())
        }
    }
}
