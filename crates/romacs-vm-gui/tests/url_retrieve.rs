//! url.el's HTTP retrieval in `romacs-vm-gui -Q --batch` against pinned GNU's
//! `emacs -Q --batch`: two `url-retrieve-synchronously' calls to a local
//! keep-alive server, so the second goes through url-http's pool of free
//! connections, which url-http.el keys by `main-thread'.  Both print each
//! response's status line and body.
#[path = "support/installed.rs"]
mod installed;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;

/// Answer every request on every connection with 200 and its path.
fn serve(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            loop {
                let mut request = String::new();
                if reader.read_line(&mut request).unwrap_or(0) == 0 { return; }
                let path = request.split(' ').nth(1).unwrap_or("/").to_owned();
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" { line.clear(); }
                let body = format!("served {path}\n");
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}", body.len());
                if writer.write_all(response.as_bytes()).is_err() { return; }
            }
        });
    }
}

#[test]
fn url_retrieve_through_the_connection_pool_matches_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || serve(listener));
    let dir = std::env::temp_dir().join(format!("romacs-url-retrieve-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = format!(
        "(progn (require 'url)
         (let ((url-proxy-services nil))
           (dolist (path '(\"/one\" \"/two\"))
             (with-current-buffer (url-retrieve-synchronously (format \"http://127.0.0.1:{port}%s\" path) t t 10)
               (goto-char (point-min))
               (princ (format \"%s|%s\" (buffer-substring (point) (line-end-position))
                              (progn (re-search-forward \"\\n\\n\") (buffer-substring (point) (point-max)))))))))");
    let run = |binary: &str| {
        let output = Command::new(binary).args(["-Q", "--batch", "--eval", &form]).current_dir(&dir)
            .env("HOME", &dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
            .output().unwrap();
        (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(),
         String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default().to_owned())
    };
    let expected = run(&gnu);
    assert_eq!(expected.1, "HTTP/1.1 200 OK|served /one\nHTTP/1.1 200 OK|served /two\n", "pinned GNU: {expected:?}");
    let actual = run(romacs);
    assert_eq!((actual.0, &actual.1), (expected.0, &expected.1), "Romacs stderr: {}", actual.2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// "served gzip\n", gzip-compressed (mtime 0).
const GZIPPED: [u8; 32] = [31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 43, 78, 45, 42, 75, 77, 81, 72, 175, 202, 44, 224, 2, 0, 22, 8, 165, 63, 12, 0, 0, 0];

/// Answer one request: gzip-encoded when it accepts gzip, else plain.
fn serve_encoded(listener: TcpListener) {
    let Ok((stream, _)) = listener.accept() else { return };
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;
    let (mut line, mut gzip) = (String::new(), false);
    while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
        gzip |= line.to_ascii_lowercase().starts_with("accept-encoding:") && line.contains("gzip");
        line.clear();
    }
    let (encoding, body): (&str, &[u8]) = if gzip { ("Content-Encoding: gzip\r\n", &GZIPPED) } else { ("", b"plain\n") };
    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n{encoding}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = writer.write_all(head.as_bytes()).and_then(|()| writer.write_all(body));
}

/// url-vars.el asks for gzip when `zlib-available-p' says so, and url-http
/// decompresses the body with `zlib-decompress-region' (decompress.c).
#[test]
fn url_retrieve_decompresses_a_gzip_body_as_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-url-gzip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |binary: &str| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || serve_encoded(listener));
        let form = format!(
            "(progn (require 'url)
             (let ((url-proxy-services nil))
               (with-current-buffer (url-retrieve-synchronously \"http://127.0.0.1:{port}/z\" t t 10)
                 (goto-char (point-min))
                 (re-search-forward \"\\n\\n\")
                 (princ (format \"%S|%s\" url-mime-encoding-string (buffer-substring (point) (point-max)))))))");
        let output = Command::new(binary).args(["-Q", "--batch", "--eval", &form]).current_dir(&dir)
            .env("HOME", &dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
            .output().unwrap();
        server.join().unwrap();
        (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(),
         String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default().to_owned())
    };
    let expected = run(&gnu);
    assert_eq!(expected.1, "\"gzip\"|served gzip\n", "pinned GNU: {expected:?}");
    let actual = run(romacs);
    assert_eq!((actual.0, &actual.1), (expected.0, &expected.1), "Romacs stderr: {}", actual.2);
    let _ = std::fs::remove_dir_all(&dir);
}
