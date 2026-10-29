// Standalone worker peer, compiled with rustc by behavioral tests.
use std::{io::{BufRead,Write}, path::Path};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let model = Path::new(&args[2]);
    let mode = std::fs::read_to_string(model.join("scenario")).unwrap_or_default();
    match mode.trim() {
        "load-error" => { println!("{{\"error\":\"model rejected\"}}"); return; }
        "not-ready" => { println!("{{\"ready\":false}}"); return; }
        "invalid" => { println!("invalid JSON"); return; }
        "exit" => return,
        _ => (),
    }
    println!("{{\"ready\":true,\"device\":\"cpu\",\"load_ms\":1}}");
    std::io::stdout().flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap(); std::fs::write(model.join("request.json"), &line).unwrap();
        if mode.trim() == "slow" { std::thread::sleep(std::time::Duration::from_secs(60)); }
        if mode.trim() == "crash" { std::process::exit(1); }
        if mode.trim() == "error" { println!("{{\"error\":\"bad clip\"}}"); }
        else if mode.trim() == "empty" { println!("{{\"text\":\"\",\"device\":\"cpu\",\"inference_ms\":1}}"); }
        else { println!("{{\"text\":\"get hub\",\"device\":\"cpu\",\"inference_ms\":1}}"); }
        std::io::stdout().flush().unwrap();
    }
}
