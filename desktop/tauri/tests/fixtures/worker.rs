// Standalone worker peer, compiled with rustc by behavioral tests.
use std::{io::{BufRead,Write}, path::Path};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args[1].ends_with("finetune.py") {
        let root = std::path::PathBuf::from(std::env::var_os("SPEECHIFY_TAURI_TRAINING_HOME").unwrap());
        let report = &args[args.iter().position(|a| a=="--report").unwrap()+1];
        std::fs::write(root.join("trainer-args.txt"), args.join("\n")).unwrap();
        let result = std::fs::read_to_string(root.join("trainer-result.json")).unwrap();
        if result=="exit" { std::process::exit(1); }
        std::fs::write(report,result).unwrap();
        return;
    }
    let model = Path::new(&args[2]);
    std::fs::write(model.join("started"), "ready to run").unwrap();
    let mode = std::fs::read_to_string(model.join("scenario")).unwrap_or_default();
    match mode.trim() {
        "load-error" => { println!("{{\"error\":\"model rejected\"}}"); return; }
        "not-ready" => { println!("{{\"ready\":false}}"); return; }
        "invalid" => { println!("invalid JSON"); return; }
        "invalid-utf8" => { std::io::stdout().write_all(b"\xff\n").unwrap(); std::io::stdout().flush().unwrap(); return; }
        "exit" => return,
        _ => (),
    }
    println!("{{\"ready\":true,\"device\":\"cpu\",\"load_ms\":1}}");
    std::io::stdout().flush().unwrap();
    if mode.trim() == "burst" {
        for _ in 0..1000 { println!("{{\"text\":\"unexpected peer output\"}}"); }
        std::io::stdout().flush().unwrap();
    }
    if mode.trim() == "exit-after-ready" { return; }
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap(); std::fs::write(model.join("request.json"), &line).unwrap();
        if mode.trim() == "slow" { std::thread::sleep(std::time::Duration::from_secs(60)); }
        if mode.trim() == "crash" { std::process::exit(1); }
        if mode.trim() == "error" || mode.trim()=="gpu-error" && args[3]!="cpu" { println!("{{\"error\":\"bad clip\"}}"); }
        else if mode.trim() == "empty" { println!("{{\"text\":\"\",\"device\":\"cpu\",\"inference_ms\":1}}"); }
        else { println!("{{\"text\":\"get hub\",\"device\":\"cpu\",\"inference_ms\":1}}"); }
        std::io::stdout().flush().unwrap();
    }
}
