//! HTTP-Client für Server mit Anthropic-Messages-Schnittstelle
//! (`POST /v1/messages`, `GET /v1/models`): oMLX, Splash, Ollama.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Error, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Lokale Modelle brauchen für lange Mails und Denkphasen durchaus Minuten.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const ANTHROPIC_URL: &str = "https://api.anthropic.com";

pub struct MessagesClient {
    http: reqwest::Client,
    base: Url,
    api_key: Option<String>,
    /// Manche Server lehnen `thinking` ab; dann ohne weiter.
    send_thinking: AtomicBool,
    /// Anthropic-Cloud: nur `x-api-key`, kein `thinking: disabled`, keine `temperature`
    /// (die aktuellen Claude-Modelle lehnen beides ab).
    anthropic_cloud: bool,
}

#[derive(Debug, Clone)]
pub struct Completion<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub user: &'a str,
    pub max_tokens: u32,
}

/// Prüft und normalisiert die Server-URL (nur http/https, ohne Pfad-Rest).
pub fn parse_base_url(url: &str) -> Result<Url> {
    let url =
        Url::parse(url.trim()).map_err(|e| Error::Invalid(format!("Server-URL „{url}“: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(Error::Invalid(
            "Server-URL muss mit http:// oder https:// beginnen".into(),
        ));
    }
    Ok(url)
}

/// Läuft der Server auf diesem Rechner? Sonst verlassen Mail-Inhalte das Gerät.
pub fn is_local(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_lowercase();
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => host == "localhost" || host.ends_with(".localhost"),
    }
}

impl MessagesClient {
    pub fn new(base_url: &str, api_key: Option<String>) -> Result<Self> {
        let base = parse_base_url(base_url)?;
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(crate::tls::client_config()?)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| Error::Ai(format!("HTTP-Client: {e}")))?;
        let anthropic_cloud = base
            .host_str()
            .is_some_and(|h| h.eq_ignore_ascii_case("api.anthropic.com"));
        Ok(Self {
            http,
            base,
            api_key: api_key.filter(|k| !k.is_empty()),
            send_thinking: AtomicBool::new(!anthropic_cloud),
            anthropic_cloud,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        let base = self.base.as_str().trim_end_matches('/');
        Url::parse(&format!("{base}{path}")).map_err(|e| Error::Invalid(e.to_string()))
    }

    fn authorized(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(key) if self.anthropic_cloud => req.header("x-api-key", key),
            // oMLX/Splash prüfen Bearer oder x-api-key; beides schadet nicht.
            Some(key) => req.bearer_auth(key).header("x-api-key", key),
            None => req,
        }
    }

    fn send_error(&self, e: reqwest::Error) -> Error {
        if e.is_connect() {
            Error::Ai(format!(
                "Server unter {} nicht erreichbar. Läuft oMLX, Splash oder Ollama?",
                self.base
            ))
        } else if e.is_timeout() {
            Error::Ai("Der Server hat nicht rechtzeitig geantwortet".into())
        } else {
            Error::Ai(e.to_string())
        }
    }

    async fn check(&self, path: &str, resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        let detail: String = body.chars().take(300).collect();
        Err(Error::Ai(match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                "API-Schlüssel fehlt oder ist falsch".into()
            }
            StatusCode::NOT_FOUND if path == "/v1/messages" => format!(
                "{path} gibt es auf diesem Server nicht (Ollama erst ab einer aktuellen Version). {detail}"
            ),
            _ => format!("HTTP {status} bei {path}: {detail}"),
        }))
    }

    /// Modell-IDs aus `GET /v1/models`, sortiert.
    pub async fn models(&self) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Models {
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }

        let path = "/v1/models";
        let resp = self
            .authorized(self.http.get(self.endpoint(path)?))
            .send()
            .await
            .map_err(|e| self.send_error(e))?;
        let models: Models = self
            .check(path, resp)
            .await?
            .json()
            .await
            .map_err(|e| Error::Ai(format!("Antwort von {path} nicht lesbar: {e}")))?;
        let mut ids: Vec<String> = models.data.into_iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Schickt eine Anfrage an `/v1/messages` und liefert den Text der Antwort
    /// (nur `text`-Blöcke, Denk-Blöcke werden ausgelassen).
    pub async fn complete(&self, c: &Completion<'_>) -> Result<String> {
        let path = "/v1/messages";
        let mut body = json!({
            "model": c.model,
            "max_tokens": c.max_tokens,
            "system": c.system,
            "messages": [{ "role": "user", "content": c.user }],
        });
        if !self.anthropic_cloud {
            body["temperature"] = json!(0);
        }
        loop {
            let thinking = self.send_thinking.load(Ordering::Relaxed);
            if thinking {
                body["thinking"] = json!({ "type": "disabled" });
            } else if let Some(obj) = body.as_object_mut() {
                obj.remove("thinking");
            }
            let resp = self
                .authorized(self.http.post(self.endpoint(path)?))
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(&body)
                .send()
                .await
                .map_err(|e| self.send_error(e))?;

            if thinking && resp.status() == StatusCode::BAD_REQUEST {
                let text = resp.text().await.unwrap_or_default();
                if text.to_lowercase().contains("thinking") {
                    self.send_thinking.store(false, Ordering::Relaxed);
                    continue;
                }
                let detail: String = text.chars().take(300).collect();
                return Err(Error::Ai(format!("HTTP 400 bei {path}: {detail}")));
            }

            let value: Value = self
                .check(path, resp)
                .await?
                .json()
                .await
                .map_err(|e| Error::Ai(format!("Antwort von {path} nicht lesbar: {e}")))?;
            return Ok(text_blocks(&value));
        }
    }
}

fn text_blocks(value: &Value) -> String {
    value["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Minimaler HTTP-Server: beantwortet Anfragen der Reihe nach mit den
    /// vorgegebenen (Status, JSON) und merkt sich die Anfragen.
    async fn serve(responses: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = sock.read(&mut chunk).await.unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let len = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if buf.len() >= head_end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).to_string());
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (url, seen)
    }

    fn request<'a>() -> Completion<'a> {
        Completion {
            model: "m",
            system: "sys",
            user: "hallo",
            max_tokens: 100,
        }
    }

    #[tokio::test]
    async fn complete_returns_text_blocks_and_sends_key() {
        let (url, seen) = serve(vec![(
            200,
            r#"{"content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"{\"a\":"},{"type":"text","text":"1}"}]}"#,
        )])
        .await;
        let client = MessagesClient::new(&url, Some("geheim".into())).unwrap();
        assert_eq!(client.complete(&request()).await.unwrap(), r#"{"a":1}"#);

        let req = seen.lock().unwrap()[0].clone();
        assert!(req.starts_with("POST /v1/messages "));
        assert!(req.to_lowercase().contains("x-api-key: geheim"));
        assert!(req.contains(r#""thinking":{"type":"disabled"}"#));
        assert!(req.contains(r#""system":"sys""#));
    }

    #[tokio::test]
    async fn retries_without_thinking_when_rejected() {
        let (url, seen) = serve(vec![
            (400, r#"{"error":"unknown field: thinking"}"#),
            (200, r#"{"content":[{"type":"text","text":"ok"}]}"#),
        ])
        .await;
        let client = MessagesClient::new(&url, None).unwrap();
        assert_eq!(client.complete(&request()).await.unwrap(), "ok");
        let log = seen.lock().unwrap();
        assert!(!log[1].contains("thinking"));
        assert!(!log[0].to_lowercase().contains("x-api-key"));
    }

    #[tokio::test]
    async fn lists_models_and_reports_auth_errors() {
        let (url, _) = serve(vec![
            (200, r#"{"data":[{"id":"qwen-b"},{"id":"qwen-a"}]}"#),
            (401, r#"{"error":"no key"}"#),
        ])
        .await;
        let client = MessagesClient::new(&url, None).unwrap();
        assert_eq!(client.models().await.unwrap(), ["qwen-a", "qwen-b"]);
        let err = client.models().await.unwrap_err().to_string();
        assert!(err.contains("API-Schlüssel"), "{err}");
    }

    #[tokio::test]
    async fn unreachable_server_has_clear_message() {
        // Port eines gerade geschlossenen Listeners: Verbindung wird abgewiesen.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let err = MessagesClient::new(&url, None)
            .unwrap()
            .models()
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("nicht erreichbar"), "{err}");
    }

    #[test]
    fn anthropic_cloud_is_detected() {
        let c = MessagesClient::new(ANTHROPIC_URL, Some("k".into())).unwrap();
        assert!(c.anthropic_cloud);
        assert!(!c.send_thinking.load(Ordering::Relaxed));
        assert!(!is_local(&parse_base_url(ANTHROPIC_URL).unwrap()));
        let local = MessagesClient::new("http://127.0.0.1:8000", None).unwrap();
        assert!(!local.anthropic_cloud);
    }

    #[test]
    fn url_checks() {
        assert!(parse_base_url("ftp://x").is_err());
        assert!(parse_base_url("localhost:8000").is_err());
        assert!(is_local(&parse_base_url("http://127.0.0.1:8000").unwrap()));
        assert!(is_local(&parse_base_url("http://localhost:11434").unwrap()));
        assert!(is_local(&parse_base_url("http://[::1]:8000").unwrap()));
        assert!(!is_local(
            &parse_base_url("https://ai.example.org").unwrap()
        ));
        assert!(!is_local(
            &parse_base_url("http://192.168.1.10:8000").unwrap()
        ));
    }
}
