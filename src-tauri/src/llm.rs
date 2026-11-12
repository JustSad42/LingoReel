//! Phrase explanations from a model running on this machine. Either the bundled
//! llama.cpp server (downloaded in Setup) or any OpenAI-compatible local endpoint
//! the user points us at (Ollama, LM Studio). Nothing is ever sent to a cloud service.
use crate::align;
use crate::components;
use crate::db;
use crate::media;
use crate::util::{cmd, Res};
use crate::AppState;
use anyhow::{anyhow, bail, Context};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;
use tauri::State;

pub struct Server {
    child: tokio::process::Child,
    port: u16,
}

impl Server {
    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn custom_endpoint(state: &AppState) -> Option<(String, String)> {
    let db = state.db.lock().unwrap();
    let url = db::setting(&db, "llmUrl").as_str().unwrap_or("").trim().trim_end_matches('/').to_string();
    let model = db::setting(&db, "llmModel").as_str().unwrap_or("").to_string();
    (!url.is_empty()).then_some((url, model))
}

#[derive(Serialize)]
pub struct LlmStatus {
    pub available: bool,
    pub managed: bool,
}

#[tauri::command]
pub fn llm_status(state: State<AppState>) -> LlmStatus {
    let managed = components::locate(&state, "llama").is_some() && components::locate(&state, "llm-model").is_some();
    LlmStatus { available: managed || custom_endpoint(&state).is_some(), managed }
}

/// Base URL of a ready chat endpoint, starting our own server on first use.
async fn endpoint(state: &AppState) -> anyhow::Result<(String, String)> {
    if let Some(custom) = custom_endpoint(state) {
        return Ok(custom);
    }
    let mut guard = state.llm.lock().await;
    if let Some(s) = guard.as_mut() {
        if matches!(s.child.try_wait(), Ok(None)) {
            return Ok((format!("http://127.0.0.1:{}/v1", s.port), String::new()));
        }
    }
    let exe = components::require(state, "llama")?;
    let model = components::require(state, "llm-model")?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let child = cmd(&exe)
        .current_dir(exe.parent().unwrap())
        .arg("-m").arg(&model)
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "-ngl", "99", "-c", "4096"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the local LLM")?;
    *guard = Some(Server { child, port });
    // Loading a few GB of weights takes a while the first time.
    for _ in 0..240 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(s) = guard.as_mut() {
            if !matches!(s.child.try_wait(), Ok(None)) {
                *guard = None;
                bail!("the local LLM stopped while loading its model");
            }
        }
        let ok = state
            .http
            .get(format!("http://127.0.0.1:{port}/health"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if ok {
            return Ok((format!("http://127.0.0.1:{port}/v1"), String::new()));
        }
    }
    bail!("the local LLM took too long to start")
}

async fn chat(state: &AppState, system: &str, user: &str) -> anyhow::Result<String> {
    let (base, model) = endpoint(state).await?;
    let mut body = json!({
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "temperature": 0.3,
        "max_tokens": 500,
        "stream": false,
    });
    if !model.is_empty() {
        body["model"] = json!(model);
    }
    let resp: Value = state
        .http
        .post(format!("{base}/chat/completions"))
        .timeout(Duration::from_secs(180))
        .json(&body)
        .send()
        .await
        .context("could not reach the local LLM")?
        .error_for_status()?
        .json()
        .await?;
    resp["choices"][0]["message"]["content"]
        .as_str()
        // Small models add markdown emphasis even when told not to.
        .map(|s| s.replace('*', "").trim().to_string())
        .ok_or_else(|| anyhow!("the local LLM returned an empty answer"))
}

fn lang_name(code: &str) -> &str {
    match code {
        "fr" => "French",
        "ja" => "Japanese",
        "es" => "Spanish",
        "de" => "German",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ko" => "Korean",
        "zh" => "Chinese",
        other => other,
    }
}

/// Explain a word or phrase in the context of its line, or translate a whole line.
#[tauri::command]
pub async fn llm_explain(state: State<'_, AppState>, lang: String, text: String, context: String, mode: String) -> Res<String> {
    let name = lang_name(&lang);
    let system = format!(
        "You are a concise {name} tutor for an English-speaking beginner. Answer in English, in plain text without markdown."
    );
    let user = if mode == "translate" {
        format!("Translate this {name} film line into natural English. Reply with only the translation.\n\n{text}")
    } else {
        format!(
            "Line from a film: \"{context}\"\n\nExplain this part: \"{text}\"\n\nGive: (1) a natural English translation, \
             (2) a short word-by-word breakdown, (3) any idiom, grammar point or register a beginner should notice. \
             Stay under 120 words."
        )
    };
    Ok(chat(&state, &system, &user).await?)
}

#[tauri::command]
pub async fn llm_stop(state: State<'_, AppState>) -> Res<()> {
    if let Some(mut s) = state.llm.lock().await.take() {
        s.kill();
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ShadowResult {
    pub heard: String,
    /// Each expected word and whether it was heard.
    pub words: Vec<(String, bool)>,
    pub score: f64,
}

/// Shadowing: transcribe the learner's recording and compare it to the line.
#[tauri::command]
pub async fn shadow_score(state: State<'_, AppState>, audio: Vec<u8>, expected: String, lang: String) -> Res<ShadowResult> {
    let tmp = state.dir.join("tmp").join(format!("shadow-{}", std::process::id()));
    tokio::fs::create_dir_all(&tmp).await?;
    let (src, wav) = (tmp.join("rec.webm"), tmp.join("rec.wav"));
    tokio::fs::write(&src, &audio).await?;
    let ffmpeg = components::require(&state, "ffmpeg")?;
    let out = cmd(ffmpeg)
        .args(["-y", "-v", "error", "-nostdin", "-i"])
        .arg(&src)
        .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .output()
        .await?;
    if !out.status.success() {
        return Err(anyhow!("could not read the recording: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    let _busy = state.prepare_lock.lock().await;
    let heard = media::whisper_words(&state, &wav, &lang, &tmp.join("rec"), |_| {}).await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let heard_text = heard.iter().map(|h| h.text.trim()).collect::<Vec<_>>().join(" ");

    let mut bag: HashMap<String, i32> = HashMap::new();
    for k in heard.iter().flat_map(|h| align::keys(&h.text)) {
        *bag.entry(k).or_default() += 1;
    }
    let mut words = vec![];
    let mut hit = 0usize;
    for w in expected.split_whitespace() {
        let ks = align::keys(w);
        if ks.is_empty() {
            continue;
        }
        let ok = ks.iter().all(|k| bag.get(k).copied().unwrap_or(0) > 0);
        if ok {
            hit += 1;
            for k in &ks {
                *bag.get_mut(k).unwrap() -= 1;
            }
        }
        words.push((w.to_string(), ok));
    }
    let score = if words.is_empty() { 0.0 } else { hit as f64 / words.len() as f64 };
    Ok(ShadowResult { heard: heard_text, words, score })
}

/// Generic JSON request, used by the UI to talk to Plex without browser CORS limits.
#[tauri::command]
pub async fn http_json(
    state: State<'_, AppState>,
    method: String,
    url: String,
    headers: HashMap<String, String>,
    timeout_ms: Option<u64>,
) -> Res<Value> {
    let m = reqwest::Method::from_bytes(method.as_bytes())?;
    let mut req = state.http.request(m, &url).header("Accept", "application/json").timeout(Duration::from_millis(timeout_ms.unwrap_or(20_000)));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.send().await?.error_for_status()?;
    let text = resp.text().await?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}
