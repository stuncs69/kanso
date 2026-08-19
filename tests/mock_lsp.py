import json
import sys


def send(obj):
    body = json.dumps(obj).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()


def read_msg():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1])
    if length is None:
        return None
    return json.loads(sys.stdin.buffer.read(length))


saved = False

while True:
    msg = read_msg()
    if msg is None:
        break
    method = msg.get("method", "")
    mid = msg.get("id")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {"capabilities": {
                "diagnosticProvider": {
                    "interFileDependencies": False,
                    "workspaceDiagnostics": False,
                },
            }},
        })
        send({
            "jsonrpc": "2.0",
            "id": 9999,
            "method": "workspace/configuration",
            "params": {"items": [{}, {}]},
        })
    elif method == "textDocument/didOpen":
        send({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": msg["params"]["textDocument"]["uri"],
                "diagnostics": [
                    {
                        "range": {
                            "start": {"line": 0, "character": 3},
                            "end": {"line": 0, "character": 7},
                        },
                        "severity": 1,
                        "source": "mock",
                        "message": "undeclared identifier 'main'",
                    },
                    {
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 2},
                        },
                        "severity": 2,
                        "message": "unused",
                    },
                ],
            },
        })
    elif method == "textDocument/didSave":
        saved = True
    elif method == "textDocument/diagnostic":
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {
                "kind": "full",
                "items": [
                    {
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 2},
                        },
                        "severity": 1,
                        "message": "after save" if saved else "pulled",
                    },
                ],
            },
        })
    elif method == "textDocument/completion":
        items = [
            {"label": "mock_alpha"},
            {"label": "mock_beta", "insertText": "mock_beta"},
            {"label": "extra", "textEdit": {"newText": "mock_gamma", "range": {}}},
        ]
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {"isIncomplete": False, "items": items},
        })
    elif method == "textDocument/hover":
        value = "```rust\nfn mock()\n```\n\nMock hover docs"
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {"contents": {"kind": "markdown", "value": value}},
        })
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": mid, "result": None})
    elif method == "exit":
        break
