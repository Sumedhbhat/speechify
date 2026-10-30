use super::*;
#[test]
fn every_worker_captured_setting_invalidates_loaded_model() {
    let original = config::defaults();
    for (key, value) in [
        ("model", json!("moonshine-base")),
        ("device", json!("cpu")),
        ("onnx_threads", json!(7)),
        ("compute_type", json!("int8")),
        ("language", json!("fr")),
        ("beam_size", json!(7)),
    ] {
        let mut changed = original.clone();
        changed[key] = value;
        if changed[key] != original[key] {
            assert_ne!(model_key(&changed), model_key(&original), "{key}");
        }
    }
    let mut changed = original.clone();
    changed["theme"] = json!("light");
    assert_eq!(model_key(&changed), model_key(&original));
}
