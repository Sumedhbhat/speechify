use super::*;
#[test]
fn auto_selection_respects_memory_and_cpu_consent() {
    assert!(choose_teacher("auto", None, false).is_err());
    assert_eq!(
        choose_teacher("auto", Some(100.), false).unwrap().0,
        "granite-speech-4.1-2b"
    );
    assert_eq!(
        choose_teacher("auto", None, true).unwrap(),
        ("qwen3-asr-1.7b".into(), "cpu")
    );
    assert!(choose_teacher("auto", Some(0.1), false).is_err());
}
