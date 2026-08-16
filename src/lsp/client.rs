use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

pub enum Reply {
    None,
    Ready,
    Failed(String),
    Completion { id: u64, items: Vec<String> },
    Hover { id: u64, text: Option<String> },
}

enum Pending {
    Initialize,
    Completion,
    Hover,
}

pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    latest_completion: u64,
    latest_hover: u64,
    pub ready: bool,
    pub name: String,
    language_id: String,
    documents: HashMap<String, u64>,
}

pub fn spawn(
    command_line: &str,
    file_path: &Path,
    language_id: &str,
    forward: impl Fn(Value) + Send + 'static,
) -> io::Result<LspClient> {
    let mut parts = command_line.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty server command"))?;
    let mut child = Command::new(program)
        .args(parts)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stdin = child.stdin.take().expect("stdin was piped");
    std::thread::spawn(move || {
        read_loop(stdout, &forward);
        forward(Value::Null);
    });

    let name = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string());
    let root = project_root(file_path);
    let mut client = LspClient {
        child,
        stdin,
        next_id: 0,
        pending: HashMap::new(),
        latest_completion: 0,
        latest_hover: 0,
        ready: false,
        name,
        language_id: language_id.to_string(),
        documents: HashMap::new(),
    };
    let root_uri = file_uri(&root);
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    client.request(
        "initialize",
        json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "completion": { "completionItem": { "snippetSupport": false } },
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                },
            },
            "workspaceFolders": [{ "uri": root_uri, "name": root_name }],
        }),
        Pending::Initialize,
    )?;
    Ok(client)
}

impl LspClient {
    pub fn handle_message(&mut self, msg: Value) -> io::Result<Reply> {
        if msg.is_null() {
            return Ok(Reply::Failed(format!("{} exited", self.name)));
        }
        let Some(id) = msg.get("id").cloned() else {
            return Ok(Reply::None);
        };
        if msg.get("method").is_some() {
            self.answer_server_request(&msg, id)?;
            return Ok(Reply::None);
        }
        let Some(id) = id.as_u64() else {
            return Ok(Reply::None);
        };
        let Some(pending) = self.pending.remove(&id) else {
            return Ok(Reply::None);
        };
        if let Some(error) = msg.get("error") {
            if matches!(pending, Pending::Initialize) {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("initialize failed");
                return Ok(Reply::Failed(format!("{}: {message}", self.name)));
            }
        }
        let result = msg.get("result").cloned().unwrap_or(Value::Null);
        match pending {
            Pending::Initialize => {
                self.notify("initialized", json!({}))?;
                self.ready = true;
                Ok(Reply::Ready)
            }
            Pending::Completion => {
                if id != self.latest_completion {
                    return Ok(Reply::None);
                }
                Ok(Reply::Completion {
                    id,
                    items: parse_completion_items(&result),
                })
            }
            Pending::Hover => {
                if id != self.latest_hover {
                    return Ok(Reply::None);
                }
                Ok(Reply::Hover {
                    id,
                    text: parse_hover(&result),
                })
            }
        }
    }

    fn answer_server_request(&mut self, msg: &Value, id: Value) -> io::Result<()> {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "workspace/configuration" => {
                let count = msg
                    .pointer("/params/items")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                Value::Array(vec![Value::Null; count])
            }
            _ => Value::Null,
        };
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    pub fn is_open(&self, uri: &str) -> bool {
        self.documents.contains_key(uri)
    }

    pub fn did_open(&mut self, uri: &str, text: &str) -> io::Result<()> {
        self.documents.insert(uri.to_string(), 1);
        let params = json!({
            "textDocument": {
                "uri": uri,
                "languageId": self.language_id,
                "version": 1,
                "text": text,
            },
        });
        self.notify("textDocument/didOpen", params)
    }

    pub fn did_change(&mut self, uri: &str, text: &str) -> io::Result<()> {
        let version = self.documents.entry(uri.to_string()).or_insert(0);
        *version += 1;
        let version = *version;
        let params = json!({
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [{ "text": text }],
        });
        self.notify("textDocument/didChange", params)
    }

    pub fn completion(&mut self, uri: &str, line: usize, character: usize) -> io::Result<u64> {
        let params = position_params(uri, line, character);
        let id = self.request("textDocument/completion", params, Pending::Completion)?;
        self.latest_completion = id;
        Ok(id)
    }

    pub fn hover(&mut self, uri: &str, line: usize, character: usize) -> io::Result<u64> {
        let params = position_params(uri, line, character);
        let id = self.request("textDocument/hover", params, Pending::Hover)?;
        self.latest_hover = id;
        Ok(id)
    }

    pub fn shutdown(&mut self) {
        let _ = self.request("shutdown", Value::Null, Pending::Initialize);
        let _ = self.notify("exit", Value::Null);
    }

    fn request(&mut self, method: &str, params: Value, pending: Pending) -> io::Result<u64> {
        self.next_id += 1;
        let id = self.next_id;
        self.pending.insert(id, pending);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        Ok(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> io::Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&mut self, value: &Value) -> io::Result<()> {
        let body = value.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len())?;
        self.stdin.flush()
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn position_params(uri: &str, line: usize, character: usize) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character },
    })
}

fn read_loop(stdout: ChildStdout, forward: &impl Fn(Value)) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().ok();
                }
            }
        }
        let Some(length) = content_length else {
            return;
        };
        let mut body = vec![0u8; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&body) {
            forward(value);
        }
    }
}

fn parse_completion_items(result: &Value) -> Vec<String> {
    let items = result
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| result.as_array());
    let Some(items) = items else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let text = item
            .pointer("/textEdit/newText")
            .and_then(Value::as_str)
            .or_else(|| item.get("insertText").and_then(Value::as_str))
            .or_else(|| item.get("label").and_then(Value::as_str));
        if let Some(text) = text {
            if !text.is_empty() {
                out.push(text.to_string());
            }
        }
        if out.len() >= 100 {
            break;
        }
    }
    out
}

fn parse_hover(result: &Value) -> Option<String> {
    let contents = result.get("contents")?;
    let text = if let Some(s) = contents.as_str() {
        s.to_string()
    } else if let Some(s) = contents.get("value").and_then(Value::as_str) {
        s.to_string()
    } else {
        contents
            .as_array()?
            .iter()
            .filter_map(|part| {
                part.as_str()
                    .or_else(|| part.get("value").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub fn file_uri(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    format!("file://{}", absolute.display()).replace(' ', "%20")
}

const ROOT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "go.mod",
    "package.json",
    "pyproject.toml",
    "compile_commands.json",
];

pub(crate) fn project_root(file_path: &Path) -> PathBuf {
    let absolute = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(file_path)
    };
    let start = absolute.parent().unwrap_or(&absolute);
    for dir in start.ancestors() {
        if ROOT_MARKERS.iter().any(|m| dir.join(m).exists()) {
            return dir.to_path_buf();
        }
    }
    start.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn parses_completion_item_shapes() {
        let result = json!({
            "items": [
                { "label": "alpha" },
                { "label": "b", "insertText": "beta" },
                { "label": "c", "textEdit": { "newText": "gamma" } },
            ]
        });
        assert_eq!(
            parse_completion_items(&result),
            vec!["alpha", "beta", "gamma"]
        );
        let bare = json!([{ "label": "solo" }]);
        assert_eq!(parse_completion_items(&bare), vec!["solo"]);
        assert!(parse_completion_items(&Value::Null).is_empty());
    }

    #[test]
    fn parses_hover_content_shapes() {
        let markup = json!({ "contents": { "kind": "markdown", "value": "docs" } });
        assert_eq!(parse_hover(&markup).as_deref(), Some("docs"));
        let plain = json!({ "contents": "plain" });
        assert_eq!(parse_hover(&plain).as_deref(), Some("plain"));
        let list = json!({ "contents": ["one", { "value": "two" }] });
        assert_eq!(parse_hover(&list).as_deref(), Some("one\ntwo"));
        assert_eq!(parse_hover(&json!({ "contents": "  " })), None);
        assert_eq!(parse_hover(&Value::Null), None);
    }

    #[test]
    fn finds_project_root_by_marker() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lsp/client.rs");
        assert_eq!(project_root(&file), Path::new(env!("CARGO_MANIFEST_DIR")));
    }

    #[test]
    fn file_uri_is_absolute() {
        let uri = file_uri(Path::new("/tmp/a b.rs"));
        assert_eq!(uri, "file:///tmp/a%20b.rs");
    }

    #[test]
    fn full_handshake_with_mock_server() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mock_lsp.py");
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let (tx, rx) = mpsc::channel();
        let mut client = spawn(
            &format!("python3 {}", script.display()),
            &file,
            "rust",
            move |msg| {
                let _ = tx.send(msg);
            },
        )
        .expect("mock server should spawn");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut ready = false;
        let mut items = None;
        let mut hover = None;
        while Instant::now() < deadline && (items.is_none() || hover.is_none()) {
            let Ok(msg) = rx.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            match client.handle_message(msg).unwrap() {
                Reply::Ready => {
                    ready = true;
                    let uri = file_uri(&file);
                    client.did_open(&uri, "fn main() {}").unwrap();
                    client.did_change(&uri, "fn main() { }").unwrap();
                    assert!(client.is_open(&uri));
                    client.completion(&uri, 0, 3).unwrap();
                    client.hover(&uri, 0, 3).unwrap();
                }
                Reply::Completion { items: found, .. } => items = Some(found),
                Reply::Hover { text, .. } => hover = Some(text),
                Reply::Failed(e) => panic!("server failed: {e}"),
                Reply::None => {}
            }
        }
        assert!(ready);
        assert_eq!(
            items.expect("completion reply"),
            vec!["mock_alpha", "mock_beta", "mock_gamma"]
        );
        let hover = hover.expect("hover reply").expect("hover text");
        assert!(hover.contains("Mock hover docs"));
        client.shutdown();
    }
}
