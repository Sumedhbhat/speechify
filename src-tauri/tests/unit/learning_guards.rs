use super::*;
#[test]
fn both_gates_required() {
    assert!(!gates(
        &json!({"status":"accepted","gate1":{"passed":true}})
    ));
    assert!(gates(
        &json!({"status":"accepted","gate1":{"passed":true},"gate2":{"passed":true}})
    ));
}
#[test]
fn hallucination_rejected() {
    assert_eq!(judge("yes yes yes yes yes", 3.).0, 0.);
}
