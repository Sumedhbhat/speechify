use super::*;

#[test]
fn missing_or_invalid_native_libraries_cannot_make_a_model_ready() {
    let home = Home::new();
    for directory in [&home.0, &home.0.join("missing")] {
        assert!(crate::native::Native::open(directory, Path::new("model"), "cpu", 2).is_err());
    }
    std::fs::write(
        home.0.join(fixtures().native.file_name().unwrap()),
        "Not a native library",
    )
    .unwrap();
    assert!(crate::native::Native::open(&home.0, Path::new("model"), "cpu", 2).is_err());
}

#[test]
fn invalid_native_model_paths_are_refused_before_calling_the_engine() {
    let home = Home::new();
    std::fs::copy(
        &fixtures().native,
        home.0.join(fixtures().native.file_name().unwrap()),
    )
    .unwrap();
    assert!(crate::native::Native::open(&home.0, Path::new("model\0suffix"), "cpu", 2).is_err());
}

#[test]
fn incomplete_native_installations_are_rejected_before_a_model_can_become_ready() {
    let original = std::fs::read_to_string(fixtures()._home.0.join("native.rs")).unwrap();
    for missing in [
        "transcribe_abi_struct_size",
        "transcribe_abi_struct_align",
        "transcribe_backend_init_params_init",
        "transcribe_init_backends_ex",
        "transcribe_model_load_params_init",
        "transcribe_model_load_file",
        "transcribe_session_params_init",
        "transcribe_session_init",
        "transcribe_capabilities_init",
        "transcribe_model_get_capabilities",
        "transcribe_model_device",
        "transcribe_device_info_init",
        "transcribe_device_get_info",
        "transcribe_run_params_init",
        "transcribe_model_supports",
        "transcribe_whisper_run_ext_init",
        "transcribe_run",
        "transcribe_full_text",
        "transcribe_session_free",
        "transcribe_model_free",
    ] {
        let home = Home::new();
        let source = home.0.join("incomplete.rs");
        std::fs::write(
            &source,
            original.replace(missing, &format!("fixture_missing_{missing}")),
        )
        .unwrap();
        let filename = fixtures().native.file_name().unwrap();
        let mut compiler = std::process::Command::new("rustc");
        if let Ok(linker) = std::env::var("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER") {
            compiler.args(["-C", &format!("linker={linker}")]);
        }
        let compiled = compiler
            .args(["--edition=2021", "--crate-type", "cdylib"])
            .arg(source)
            .arg("-o")
            .arg(home.0.join(filename))
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let result =
            crate::native::Native::open(&home.0, Path::new("whisper-vocab-prompt"), "cpu", 2);
        assert!(
            result.is_err(),
            "model became ready despite missing {missing}"
        );
    }
}
