//! Measured Rust host prototype. Shares the exact production worker pipes.
use std::{collections::HashMap, env, fs::File, io::{self, BufRead, BufReader, Write}, os::fd::FromRawFd, time::Instant};
use serde_json::{json, Value};
fn main() {
    let args: Vec<String> = env::args().collect();
    // The driver passes these two dedicated pipe FDs with pass_fds. Rust owns
    // its duplicates; no concurrent reads or writes occur on the Python side.
    let mut worker_in = unsafe { File::from_raw_fd(args[1].parse().unwrap()) };
    let mut worker_out = BufReader::new(unsafe { File::from_raw_fd(args[2].parse().unwrap()) });
    let manifest: Value = serde_json::from_slice(&std::fs::read(&args[3]).unwrap()).unwrap();
    let mut fixtures = HashMap::new();
    for (name, path) in manifest.as_object().unwrap() {
        let bytes = std::fs::read(path.as_str().unwrap()).unwrap();
        let samples: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        let chunks: Vec<Vec<f32>> = samples.chunks(480).map(|b| b.to_vec()).collect();
        fixtures.insert(name.clone(), chunks);
    }
    println!("{{\"ready\":true}}"); io::stdout().flush().unwrap();
    for command in io::stdin().lock().lines() {
        let cmd: Value = serde_json::from_str(&command.unwrap()).unwrap();
        let chunks = &fixtures[cmd["fixture"].as_str().unwrap()];
        let start = Instant::now();
        let mut pcm = Vec::with_capacity(chunks.iter().map(|c| c.len()*4).sum());
        let mut sum = 0.0_f64;
        let mut samples = 0;
        for chunk in chunks { for &v in chunk { pcm.extend_from_slice(&v.to_le_bytes()); sum += (v*v) as f64; samples += 1; } }
        assert!(samples>0 && ((sum/samples as f64) as f32).sqrt()>=0.004);
        let audio_ns = start.elapsed().as_nanos() as u64;
        let ipc_start = Instant::now();
        serde_json::to_writer(&mut worker_in, &json!({"samples":samples,"prompt":null})).unwrap();
        worker_in.write_all(b"\n").unwrap(); worker_in.write_all(&pcm).unwrap(); worker_in.flush().unwrap();
        let mut line = String::new(); worker_out.read_line(&mut line).unwrap();
        let msg: Value = serde_json::from_str(&line).unwrap();
        assert!(msg.get("error").is_none(), "{msg}");
        let text = msg["text"].as_str().unwrap().to_owned();
        assert!(!text.is_empty());
        let ipc_ns = ipc_start.elapsed().as_nanos() as u64;
        // Default fresh profile: glossary empty, no active learned replacements,
        // optional LLM cleanup disabled. Text at this point is ready to paste.
        let ready_ns = start.elapsed().as_nanos() as u64;
        println!("{}", json!({"runtime":"Rust","text":text,"audio_ns":audio_ns,"ipc_ns":ipc_ns,"ready_ns":ready_ns,"model_ms":msg["ms"]}));
        io::stdout().flush().unwrap();
    }
}
