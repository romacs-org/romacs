//! A scripted session process, for tests that need a second one (ownership
//! is by process): commands on stdin, one line of answer each on stdout.
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::os::fd::OwnedFd;

use romacs_keeper::spawn::{End, Spec};
use romacs_keeper::Client;
use romacs_record::Handle;

fn shell(script: &str) -> Spec {
    Spec { argv: vec![b"/bin/sh".to_vec(), b"-c".to_vec(), script.as_bytes().to_vec()], env: vec![b"PATH=/usr/bin:/bin".to_vec()],
           cwd: b"/".to_vec(), umask: 0o022, fds: vec![(0, End::Null), (1, End::Writes), (2, End::Null)], ..Spec::default() }
}

fn main() {
    let socket = std::env::args().nth(1).expect("socket");
    let mut client = Client::attach(std::path::Path::new(&socket)).expect("attach");
    let mut fds: HashMap<u64, Vec<OwnedFd>> = HashMap::new();
    println!("pid {}", std::process::id());
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("line");
        let (command, rest) = line.split_once(' ').unwrap_or((&line, ""));
        let handle = |text: &str| Handle::from_raw(text.split(' ').next().unwrap_or("").parse().unwrap_or(0)).expect("handle");
        let answer = match command {
            "spawn" => client.spawn("test", 1, b"", false, &shell(rest)).map(|(row, got)| { fds.insert(row.handle.raw(), got); format!("{} {}", row.handle, row.pid) }),
            "claim" => {
                let supersedes = rest.split(' ').nth(1).and_then(|p| p.parse().ok()).unwrap_or(0);
                client.claim(handle(rest), supersedes).map(|(row, got)| { fds.insert(row.handle.raw(), got); format!("{:?}", row.status) })
            }
            "read" => {
                let count = rest.split(' ').nth(1).and_then(|n| n.parse().ok()).unwrap_or(0);
                let mut bytes = vec![0; count];
                let fd = fds[&handle(rest).raw()][0].try_clone().expect("clone");
                std::fs::File::from(fd).read_exact(&mut bytes).map(|()| String::from_utf8_lossy(&bytes).replace('\n', ",")).map_err(|e| e.to_string())
            }
            "kept" => client.set_kept(handle(rest), true).map(|()| "ok".into()),
            "quit" => client.detach(true).map(|()| "ok".into()),
            other => Err(format!("unknown command {other}")),
        };
        println!("{}", answer.unwrap_or_else(|e| format!("error {e}")));
        std::io::stdout().flush().expect("flush");
    }
}
