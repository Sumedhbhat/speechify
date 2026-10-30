use super::*;
#[test]
fn unavailable_cleanup_is_an_error_not_empty_output() {
    assert!(cleanup_text("invalid://cleanup", &json!("model"), "Keep my words.").is_err());
}
