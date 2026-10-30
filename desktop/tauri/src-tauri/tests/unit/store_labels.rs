use super::*;
#[test]
fn no_guesses() {
    assert!(label(&json!({"audio":"a.wav","pasted":"guess"}), 0.7).is_none());
    assert_eq!(label(&json!({"audio":"a.wav","corrected":"human","teacher_text":"teacher","teacher_confidence":0.9}),0.7).unwrap().0,"human");
}
#[test]
fn bounded_edits() {
    assert_eq!(
        edits("open get hub now", "open GitHub now"),
        vec![("get hub".into(), "GitHub".into())]
    );
    assert!(edits("one two three four five six", "a b c d e f").is_empty());
}
