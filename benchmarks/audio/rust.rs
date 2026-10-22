use std::{env, fs, hint::black_box, time::Instant};
fn run(a: &[f32], silence: bool) -> Vec<u8> {
    if silence {
        let sum: f64 = a.iter().map(|&v| (v * v) as f64).sum();
        return vec![(a.is_empty() || ((sum / a.len() as f64) as f32).sqrt() < 0.004) as u8];
    }
    let mut pcm = Vec::with_capacity(a.len() * 2);
    for &v in a {
        pcm.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    pcm
}
fn main() {
    let args: Vec<String> = env::args().collect();
    let bytes = fs::read(&args[1]).unwrap();
    let a: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let silence = args[2] == "silence";
    let count: u32 = args[3].parse().unwrap();
    for _ in 0..30 {
        black_box(run(black_box(&a), silence));
    }
    let start = Instant::now();
    for _ in 0..count {
        black_box(run(black_box(&a), silence));
    }
    let ns = start.elapsed().as_nanos() as f64 / count as f64;
    fs::write(&args[4], run(&a, silence)).unwrap();
    println!("{{\"ns_per_op\":{ns}}}");
}
