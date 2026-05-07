use std::{fs, io::{self, Read}, path::PathBuf};

use anyhow::Context;
use clap::{CommandFactory, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_yaml;

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
    let text = fs::read_to_string(&path)
        .with_context(|| format!("Config not found at {}. Run `qa create-account` first.", path.display()))?;
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

fn client_no_auth() -> reqwest::blocking::Client {
    reqwest::blocking::Client::new()
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
#[allow(dead_code)]
struct ChangeApiKeyResponse {
    api_key: String,
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
    #[allow(dead_code)]
    solved: bool,
    #[allow(dead_code)]
    solved_at: Option<String>,
    starred: bool,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Answer {
    id: i64,
    question_id: i64,
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

// -- CLI

#[derive(Parser)]
#[command(name = "qa", about = "Q&A system CLI", version)]
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
    
    /// List unsolved questions
    Unsolved,
    
    /// Get a question with all its answers
    Get {
        /// Question ID
        id: i64,
    },
    
    /// Answer a question (reads from stdin)
    Answer {
        /// Question ID
        id: i64,
    },
    
    /// Mark a question as solved (only the asker can do this)
    Solved {
        /// Question ID
        id: i64,
    },
    
    /// Star a question
    Star {
        /// Question ID
        id: i64,
    },
    
    /// Unstar a question
    Unstar {
        /// Question ID
        id: i64,
    },
    
    /// Generate shell completion
    Complete { shell: clap_complete::Shell },
}

// -- Main

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.cmd {
        Cmd::CreateAccount => {
            let endpoint = prompt_endpoint()?;
            let username = prompt("Username: ")?;
            
            let client = client_no_auth();
            let url = format!("{}/register", endpoint);
            
            let resp = client
                .post(&url)
                .json(&serde_json::json!({ "username": username }))
                .send()?;
            
            if resp.status().is_success() {
                let api_key = get_api_key_from_response(&resp)
                    .context("Server did not return x-api-key header")?;
                
                // Try to parse the JSON body
                let body_text = resp.text()?;
                let body_result: Result<CreateAccountResponse, _> = serde_json::from_str(&body_text);
                let body = match body_result {
                    Ok(b) => b,
                    Err(_) => {
                        // If JSON parsing fails, just extract from header we already have
                        CreateAccountResponse {
                            id: 0,
                            username: username.clone(),
                            api_key: api_key.clone(),
                        }
                    }
                };
                
                // Save config
                let cfg = ClientConfig {
                    endpoint,
                    api_key: Some(api_key.clone()),
                    username: Some(username.clone()),
                };
                save_config(&cfg)?;
                
                println!("✓ Account created: {}", body.username);
                println!("✓ API key saved to {}", config_path().display());
                println!("  API Key: {}", api_key);
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::ChangeApiKey => {
            let mut cfg = load_config()?;
            let current_api_key = cfg.api_key.as_ref()
                .context("No API key found. Run `qa create-account` first.")?;
            let endpoint = &cfg.endpoint;
            
            let client = client_with_api_key(current_api_key);
            let url = format!("{}/change-api-key", endpoint);
            
            let resp = client
                .post(&url)
                .json(&serde_json::json!({ "current_api_key": current_api_key }))
                .send()?;
            
            if resp.status().is_success() {
                let new_api_key = get_api_key_from_response(&resp)
                    .context("Server did not return x-api-key header")?;
                
                // Update and save config
                cfg.api_key = Some(new_api_key.clone());
                save_config(&cfg)?;
                
                println!("✓ API key changed successfully");
                println!("  New API Key: {}", new_api_key);
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::Ask => {
            let cfg = load_config()?;
            let api_key = cfg.api_key.as_ref()
                .context("No API key found. Run `qa create-account` first.")?;
            let endpoint = &cfg.endpoint;
            
            // Read from stdin
            let mut input = String::new();
            io::stdin().read_to_string(&mut input)?;
            
            // Parse title (first line) and content (rest)
            let lines: Vec<&str> = input.lines().collect();
            if lines.is_empty() {
                anyhow::bail!("No input provided. Expected title on first line, then content.");
            }
            
            let title = lines[0].trim().to_string();
            let content = lines[1..].join("\n").trim().to_string();
            
            if title.is_empty() {
                anyhow::bail!("Title cannot be empty");
            }
            
            let client = client_with_api_key(api_key);
            let url = format!("{}/questions", endpoint);
            
            let resp = client
                .post(&url)
                .json(&serde_json::json!({ 
                    "title": title,
                    "content": content 
                }))
                .send()?;
            
            if resp.status().is_success() {
                let question_id: i64 = resp.text()?.parse()
                    .context("Failed to parse question ID")?;
                println!("✓ Question created: #{}", question_id);
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::Unsolved => {
            let cfg = load_config()?;
            let api_key = cfg.api_key.as_ref()
                .context("No API key found. Run `qa create-account` first.")?;
            let endpoint = &cfg.endpoint;
            
            let client = client_with_api_key(api_key);
            let url = format!("{}/questions/unsolved", endpoint);
            
            let resp = client.get(&url).send()?;
            
            if resp.status().is_success() {
                let questions: Vec<Question> = resp.json()?;
                
                if questions.is_empty() {
                    println!("No unsolved questions.");
                } else {
                    for q in questions {
                        let star_marker = if q.starred { " ★" } else { "" };
                        println!("  #{} {} by {}{}", q.id, q.title, q.author, star_marker);
                        // Print first 100 chars of content
                        let preview: String = q.content.chars().take(100).collect();
                        if q.content.len() > 100 {
                            println!("    {}...", preview);
                        } else {
                            println!("    {}", preview);
                        }
                        println!();
                    }
                }
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::Get { id } => {
            let cfg = load_config()?;
            let api_key = cfg.api_key.as_ref()
                .context("No API key found. Run `qa create-account` first.")?;
            let endpoint = &cfg.endpoint;
            
            let client = client_with_api_key(api_key);
            let url = format!("{}/questions/{}", endpoint, id);
            
            let resp = client.get(&url).send()?;
            
            if resp.status().is_success() {
                let qa: QuestionWithAnswers = resp.json()?;
                let q = qa.question;
                
                let status = if q.solved { "[SOLVED]" } else { "[OPEN]" };
                let star_marker = if q.starred { " ★" } else { "" };
                
                println!("{} Question #{}: {} by {}{}", status, q.id, q.title, q.author, star_marker);
                println!("\n{}", q.content);
                println!("\n--- {} Answers ---", qa.answers.len());
                
                for ans in qa.answers {
                    println!("\nAnswer #{} by {}", ans.id, ans.author);
                    println!("{}", ans.content);
                }
            } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
                anyhow::bail!("Question #{} not found", id);
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::Answer { id } => {
            let cfg = load_config()?;
            let api_key = cfg.api_key.as_ref()
                .context("No API key found. Run `qa create-account` first.")?;
            let endpoint = &cfg.endpoint;
            
            // Read answer content from stdin
            let mut content = String::new();
            io::stdin().read_to_string(&mut content)?;
            let content = content.trim();
            
            if content.is_empty() {
                anyhow::bail!("Answer content cannot be empty");
            }
            
            let client = client_with_api_key(api_key);
            let url = format!("{}/questions/{}/answers", endpoint, id);
            
            let resp = client
                .post(&url)
                .json(&serde_json::json!({ "content": content }))
                .send()?;
            
            if resp.status().is_success() {
                let answer_id: i64 = resp.text()?.parse()
                    .context("Failed to parse answer ID")?;
                println!("✓ Answer created: #{} for question #{}" , answer_id, id);
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::Solved { id } => {
            let cfg = load_config()?;
            let api_key = cfg.api_key.as_ref()
                .context("No API key found. Run `qa create-account` first.")?;
            let endpoint = &cfg.endpoint;
            
            let client = client_with_api_key(api_key);
            let url = format!("{}/questions/{}/solved", endpoint, id);
            
            let resp = client.post(&url).send()?;
            
            if resp.status().is_success() {
                println!("✓ Question #{} marked as solved", id);
            } else if resp.status() == reqwest::StatusCode::FORBIDDEN {
                anyhow::bail!("Only the person who asked the question can mark it as solved");
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::Star { id } => {
            let cfg = load_config()?;
            let api_key = cfg.api_key.as_ref()
                .context("No API key found. Run `qa create-account` first.")?;
            let endpoint = &cfg.endpoint;
            
            let client = client_with_api_key(api_key);
            let url = format!("{}/questions/{}/star", endpoint, id);
            
            let resp = client.post(&url).send()?;
            
            if resp.status().is_success() {
                println!("✓ Starred question #{}" , id);
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::Unstar { id } => {
            let cfg = load_config()?;
            let api_key = cfg.api_key.as_ref()
                .context("No API key found. Run `qa create-account` first.")?;
            let endpoint = &cfg.endpoint;
            
            let client = client_with_api_key(api_key);
            let url = format!("{}/questions/{}/star", endpoint, id);
            
            let resp = client.delete(&url).send()?;
            
            if resp.status().is_success() {
                println!("✓ Unstarred question #{}" , id);
            } else {
                let err_text = resp.text()?;
                anyhow::bail!("Server error: {}", err_text);
            }
        }

        Cmd::Complete { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "qa", &mut std::io::stdout());
        }
    }

    Ok(())
}

fn prompt(message: &str) -> anyhow::Result<String> {
    print!("{}", message);
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn prompt_endpoint() -> anyhow::Result<String> {
    // Check if config exists and has endpoint
    let default = if let Ok(cfg) = load_config() {
        cfg.endpoint
    } else {
        "http://localhost:7878".to_string()
    };
    
    let endpoint = prompt(&format!("Server endpoint [{}]: ", default))?;
    Ok(if endpoint.is_empty() { default } else { endpoint })
}
