//! Browser mode: serve the UI and the engine API on localhost.
//!
//! `POST /api/<command>` with a JSON body → `{"ok": value}` or `{"error": msg}`.
//! `GET /media?path=…` streams project media with HTTP Range support (video
//! seeking). Only files referenced by the open project are served, and the
//! server binds to the loopback interface.

use crate::Engine;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn mime(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "wav" => "audio/wav",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 2;
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn api(engine: &Arc<Engine>, mut req: Request, cmd: &str) {
    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);
    let args: serde_json::Value =
        if body.trim().is_empty() { serde_json::Value::Null } else { serde_json::from_str(&body).unwrap_or(serde_json::Value::Null) };
    let res = match engine.call(cmd, args) {
        Ok(v) => serde_json::json!({ "ok": v }),
        Err(e) => serde_json::json!({ "error": e }),
    };
    let _ = req.respond(Response::from_string(res.to_string()).with_header(header("Content-Type", "application/json")));
}

fn media(engine: &Arc<Engine>, req: Request, path: &str) {
    let p = PathBuf::from(path);
    let ok = std::fs::canonicalize(&p).map(|c| engine.media_allowed(&c) || engine.media_allowed(&p)).unwrap_or(false);
    if !ok {
        let _ = req.respond(Response::from_string("forbidden").with_status_code(403));
        return;
    }
    let Ok(mut f) = std::fs::File::open(&p) else {
        let _ = req.respond(Response::from_string("not found").with_status_code(404));
        return;
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let range = req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.as_str().to_string());
    let ct = header("Content-Type", mime(&p));
    if let Some(r) = range.and_then(|r| r.strip_prefix("bytes=").map(String::from)) {
        let (a, b) = r.split_once('-').unwrap_or(("0", ""));
        let start: u64 = a.parse().unwrap_or(0);
        let end: u64 = b.parse().unwrap_or(len.saturating_sub(1)).min(len.saturating_sub(1)).min(start + 8 * 1024 * 1024);
        if start >= len {
            let _ = req.respond(Response::empty(416).with_header(header("Content-Range", &format!("bytes */{len}"))));
            return;
        }
        let n = (end - start + 1) as usize;
        let mut buf = vec![0u8; n];
        let _ = f.seek(SeekFrom::Start(start));
        let _ = f.read_exact(&mut buf);
        let resp = Response::from_data(buf)
            .with_status_code(StatusCode(206))
            .with_header(ct)
            .with_header(header("Accept-Ranges", "bytes"))
            .with_header(header("Content-Range", &format!("bytes {start}-{end}/{len}")));
        let _ = req.respond(resp);
    } else {
        let resp = Response::new(StatusCode(200), vec![ct, header("Accept-Ranges", "bytes")], f, Some(len as usize), None);
        let _ = req.respond(resp);
    }
}

fn static_file(ui: &Path, req: Request, url: &str) {
    let rel = url.trim_start_matches('/');
    let mut p = ui.join(if rel.is_empty() { "index.html" } else { rel });
    if !p.starts_with(ui) || rel.contains("..") || !p.is_file() {
        p = ui.join("index.html");
    }
    match std::fs::read(&p) {
        Ok(b) => {
            let _ = req.respond(Response::from_data(b).with_header(header("Content-Type", mime(&p))));
        }
        Err(_) => {
            let _ = req.respond(Response::from_string("UI not built: run `npm run build` in ui/").with_status_code(404));
        }
    }
}

struct Opts {
    api: bool,
    ui: Option<PathBuf>,
    token: Option<String>,
}

fn run_workers(server: Arc<Server>, engine: Arc<Engine>, o: Arc<Opts>, n: usize) -> Vec<std::thread::JoinHandle<()>> {
    (0..n)
        .map(|_| {
            let (server, engine, o) = (server.clone(), engine.clone(), o.clone());
            std::thread::spawn(move || {
                while let Ok(req) = server.recv() {
                    let url = req.url().to_string();
                    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
                    let param = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(url_decode);
                    match (req.method(), path) {
                        (Method::Post, p) if o.api && p.starts_with("/api/") => {
                            let cmd = p.trim_start_matches("/api/").to_string();
                            api(&engine, req, &cmd)
                        }
                        (Method::Get, "/media") => {
                            if o.token.as_ref().is_some_and(|t| param("t").as_deref() != Some(t.as_str())) {
                                let _ = req.respond(Response::from_string("forbidden").with_status_code(403));
                                continue;
                            }
                            media(&engine, req, &param("path").unwrap_or_default())
                        }
                        (Method::Get, p) if o.ui.is_some() => static_file(o.ui.as_ref().unwrap(), req, p),
                        _ => {
                            let _ = req.respond(Response::empty(404));
                        }
                    }
                }
            })
        })
        .collect()
}

/// Browser mode: serve UI + API + media until the process exits.
/// `ui_dir` is the built frontend (`ui/dist`).
pub fn serve(engine: Arc<Engine>, port: u16, ui_dir: PathBuf) -> std::io::Result<()> {
    let server = Arc::new(Server::http(("127.0.0.1", port)).map_err(|e| std::io::Error::other(e.to_string()))?);
    let ui = std::fs::canonicalize(&ui_dir).unwrap_or(ui_dir);
    for w in run_workers(server, engine, Arc::new(Opts { api: true, ui: Some(ui), token: None }), 6) {
        let _ = w.join();
    }
    Ok(())
}

/// Desktop mode: a media-only loopback server (random port + secret token)
/// so every webview engine (WebKitGTK, WKWebView, WebView2) can stream and
/// seek picture with HTTP Range requests. The engine API is NOT exposed here.
/// Returns the base URL, e.g. `http://127.0.0.1:43123/media?t=<token>`.
pub fn start_media_server(engine: Arc<Engine>) -> std::io::Result<String> {
    let server = Arc::new(Server::http(("127.0.0.1", 0)).map_err(|e| std::io::Error::other(e.to_string()))?);
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    let token = vcsd_core::ids::new_id("m")[2..].to_string();
    let base = format!("http://127.0.0.1:{port}/media?t={token}");
    run_workers(server, engine.clone(), Arc::new(Opts { api: false, ui: None, token: Some(token) }), 3);
    *engine.media_base.lock().unwrap() = Some(base.clone());
    Ok(base)
}
