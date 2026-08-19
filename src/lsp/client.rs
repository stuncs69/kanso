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
    Diagnostics { uri: String, items: Vec<Diagnostic> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    pub fn glyph(self) -> char {
        match self {
            Severity::Error => 'E',
            Severity::Warning => 'W',
            Severity::Info => 'I',
            Severity::Hint => 'H',
        }
    }

    pub fn marker(self) -> char {
        match self {
            Severity::Error => '●',
            Severity::Warning => '▲',
            Severity::Info | Severity::Hint => '·',
        }
    }

    fn from_code(code: Option<u64>) -> Severity {
        match code {
            Some(2) => Severity::Warning,
            Some(3) => Severity::Info,
            Some(4) => Severity::Hint,
            _ => Severity::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub start_line: usize,
    pub start_utf16: usize,
    pub end_line: usize,
    pub end_utf16: usize,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
}

enum Pending {
    Initialize,
    Completion,
    Hover,
    PullDiagnostics(String),
}

pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    latest_completion: u64,
    latest_hover: u64,
    latest_pull: u64,
    pull_diagnostics: bool,
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
    let debug_path = std::env::var("KANSO_DEBUG_LOG").ok();
    let mut child = Command::new(program)
        .args(parts)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if debug_path.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stdin = child.stdin.take().expect("stdin was piped");
    std::thread::spawn(move || {
        read_loop(stdout, &forward);
        forward(Value::Null);
    });
    if let (Some(path), Some(stderr)) = (debug_path, child.stderr.take()) {
        std::thread::spawn(move || log_stderr(stderr, &path));
    }

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
        latest_pull: 0,
        pull_diagnostics: false,
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
                    "diagnostic": { "dynamicRegistration": false },
                    "synchronization": { "didSave": true },
                    "publishDiagnostics": {
                        "relatedInformation": false,
                        "versionSupport": false,
                        "tagSupport": { "valueSet": [1, 2] },
                    },
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
            return Ok(self.handle_notification(&msg));
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
            return Ok(Reply::None);
        }
        let result = msg.get("result").cloned().unwrap_or(Value::Null);
        match pending {
            Pending::Initialize => {
                self.pull_diagnostics = result
                    .pointer("/capabilities/diagnosticProvider")
                    .is_some_and(|v| !v.is_null());
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
            Pending::PullDiagnostics(uri) => {
                if id != self.latest_pull
                    || result.get("kind").and_then(Value::as_str) == Some("unchanged")
                {
                    return Ok(Reply::None);
                }
                Ok(Reply::Diagnostics {
                    uri,
                    items: parse_diagnostics(result.get("items")),
                })
            }
        }
    }

    fn handle_notification(&mut self, msg: &Value) -> Reply {
        if msg.get("method").and_then(Value::as_str) != Some("textDocument/publishDiagnostics") {
            return Reply::None;
        }
        let Some(params) = msg.get("params") else {
            return Reply::None;
        };
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return Reply::None;
        };
        Reply::Diagnostics {
            uri: uri.to_string(),
            items: parse_diagnostics(params.get("diagnostics")),
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

    pub fn supports_pull(&self) -> bool {
        self.pull_diagnostics
    }

    pub fn pull_diagnostics(&mut self, uri: &str) -> io::Result<u64> {
        let params = json!({ "textDocument": { "uri": uri } });
        let id = self.request(
            "textDocument/diagnostic",
            params,
            Pending::PullDiagnostics(uri.to_string()),
        )?;
        self.latest_pull = id;
        Ok(id)
    }

    pub fn did_save(&mut self, uri: &str) -> io::Result<()> {
        let params = json!({ "textDocument": { "uri": uri } });
        self.notify("textDocument/didSave", params)
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

fn log_stderr(stderr: std::process::ChildStderr, path: &str) {
    let reader = BufReader::new(stderr);
    for line in reader.lines().map_while(Result::ok) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "stderr: {line}");
        }
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

fn parse_diagnostics(list: Option<&Value>) -> Vec<Diagnostic> {
    let Some(items) = list.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let Some(message) = item.get("message").and_then(Value::as_str) else {
            continue;
        };
        let message = message.trim();
        if message.is_empty() {
            continue;
        }
        let start_line = position_field(item, "start", "line");
        let start_utf16 = position_field(item, "start", "character");
        let end_line = position_field(item, "end", "line").max(start_line);
        let end_utf16 = position_field(item, "end", "character");
        out.push(Diagnostic {
            start_line,
            start_utf16,
            end_line,
            end_utf16,
            severity: Severity::from_code(item.get("severity").and_then(Value::as_u64)),
            message: message.to_string(),
            source: item
                .get("source")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
        if out.len() >= 500 {
            break;
        }
    }
    out.sort_by_key(|d| (d.start_line, d.start_utf16, d.severity));
    out
}

fn position_field(item: &Value, edge: &str, field: &str) -> usize {
    item.pointer(&format!("/range/{edge}/{field}"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
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
        let uri = file_uri(&file);
        let mut ready = false;
        let mut items = None;
        let mut hover = None;
        let mut published: Option<Vec<Diagnostic>> = None;
        let mut pulled = false;
        let mut pulled_after_save = false;
        while Instant::now() < deadline
            && (items.is_none() || hover.is_none() || published.is_none() || !pulled_after_save)
        {
            let Ok(msg) = rx.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            match client.handle_message(msg).unwrap() {
                Reply::Ready => {
                    ready = true;
                    assert!(client.supports_pull());
                    client.did_open(&uri, "fn main() {}").unwrap();
                    client.did_change(&uri, "fn main() { }").unwrap();
                    assert!(client.is_open(&uri));
                    client.completion(&uri, 0, 3).unwrap();
                    client.hover(&uri, 0, 3).unwrap();
                    client.pull_diagnostics(&uri).unwrap();
                }
                Reply::Completion { items: found, .. } => items = Some(found),
                Reply::Hover { text, .. } => hover = Some(text),
                Reply::Diagnostics { uri: got, items } => {
                    assert_eq!(got, uri);
                    match items.first().map(|d| d.message.as_str()) {
                        Some("pulled") => {
                            pulled = true;
                            client.did_save(&uri).unwrap();
                            client.pull_diagnostics(&uri).unwrap();
                        }
                        Some("after save") => pulled_after_save = true,
                        _ => published = Some(items),
                    }
                }
                Reply::Failed(e) => panic!("server failed: {e}"),
                Reply::None => {}
            }
        }
        assert!(ready);
        assert!(pulled);
        assert!(pulled_after_save);
        assert_eq!(
            items.expect("completion reply"),
            vec!["mock_alpha", "mock_beta", "mock_gamma"]
        );
        let hover = hover.expect("hover reply").expect("hover text");
        assert!(hover.contains("Mock hover docs"));
        let published = published.expect("diagnostics notification");
        assert_eq!(published.len(), 2);
        assert_eq!(published[0].severity, Severity::Warning);
        assert_eq!(published[0].start_utf16, 0);
        assert_eq!(published[1].severity, Severity::Error);
        assert_eq!(published[1].start_utf16, 3);
        assert_eq!(published[1].end_utf16, 7);
        assert_eq!(published[1].source.as_deref(), Some("mock"));
        client.shutdown();
    }

    #[test]
    fn clangd_publishes_diagnostics_for_broken_code() {
        if crate::lsp::default_server("cpp", Path::new("/tmp/x.cpp")).is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("kanso-clangd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("compile_commands.json"), "[]").unwrap();
        let file = dir.join("main.cpp");
        let source = "#include <iostream>\nint main() {\n  std::cout << foo;\n  return 0;\n}\n";
        std::fs::write(&file, source).unwrap();

        let (tx, rx) = mpsc::channel();
        let mut client = spawn("clangd", &file, "cpp", move |msg| {
            let _ = tx.send(msg);
        })
        .expect("clangd should spawn");

        let uri = file_uri(&file);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut published = None;
        while Instant::now() < deadline && published.is_none() {
            let Ok(msg) = rx.recv_timeout(Duration::from_millis(500)) else {
                continue;
            };
            match client.handle_message(msg).unwrap() {
                Reply::Ready => client.did_open(&uri, source).unwrap(),
                Reply::Diagnostics { uri: got, items } if got == uri && !items.is_empty() => {
                    published = Some(items)
                }
                _ => {}
            }
        }
        client.shutdown();
        let _ = std::fs::remove_dir_all(&dir);

        let published = published.expect("clangd should report the undeclared identifier");
        let error = published
            .iter()
            .find(|d| d.severity == Severity::Error)
            .expect("an error severity diagnostic");
        assert_eq!(error.start_line, 2);
        assert!(error.message.contains("foo"), "{}", error.message);
        assert!(error.end_utf16 > error.start_utf16);
    }
}
