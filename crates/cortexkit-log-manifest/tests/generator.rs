mod generator_contract {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use cortexkit_doctor::{LogSource, Manifest, ModuleId, MAX_OUTPUT_BYTES};
    use cortexkit_log_manifest::{generate, ManifestMetadata};

    const GOLDEN: &[u8] = include_bytes!("fixtures/scan/expected.json");

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scan")
    }

    fn command(crate_dir: &Path, out: Option<&Path>) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortexkit-log-manifest"));
        command.current_dir(crate_dir);
        if let Some(out) = out {
            command.arg("--out").arg(out);
        }
        command
    }

    fn metadata_flags(command: &mut Command) -> &mut Command {
        command.args([
            "--module-id",
            "fixture",
            "--repository",
            "https://github.com/example/fixture",
            "--log-glob",
            "logs/*.log",
        ])
    }

    fn assert_exit(output: Output, code: i32) -> String {
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(code), "{stderr}");
        assert!(output.stdout.is_empty());
        stderr.replace('\\', "/")
    }

    #[test]
    fn writes_golden_and_located_skips_and_checks_it() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("result.json");
        let mut write = command(tmp.path(), Some(&out));
        write.arg("--crate").arg(fixture());
        let stderr = assert_exit(metadata_flags(&mut write).output().unwrap(), 0);
        assert_eq!(fs::read(&out).unwrap(), GOLDEN);
        assert_eq!(
            stderr.lines().collect::<Vec<_>>(),
            [
                "source/lib_root.rs:16: non-constant event level",
                "source/lib_root.rs:17: message has format arguments",
                "source/lib_root.rs:18: message has format arguments or captures",
                "source/lib_root.rs:19: message starts with [",
                "source/lib_root.rs:20: non-literal target",
                "source/lib_root.rs:21: invalid target",
                "source/lib_root.rs:22: invalid message",
                "source/lib_root.rs:23: explicit message field",
                "source/lib_root.rs:24: no literal message",
                "source/lib_root.rs:26: inside a macro_rules! body",
                "source/lib_root.rs:29: inside another macro invocation",
            ]
        );
        let check = assert_exit(
            command(&fixture(), Some(&out))
                .arg("--check")
                .output()
                .unwrap(),
            0,
        );
        assert_eq!(check, stderr);
        assert_exit(
            metadata_flags(command(&fixture(), Some(&out)).arg("--check"))
                .output()
                .unwrap(),
            0,
        );
        let mismatched = assert_exit(
            command(&fixture(), Some(&out))
                .args(["--check", "--module-id", "other-fixture"])
                .output()
                .unwrap(),
            1,
        );
        assert!(mismatched.contains("metadata flags differ"));
        assert_eq!(fs::read(&out).unwrap(), GOLDEN);
    }

    #[test]
    fn check_rejects_stale_tampered_unterminated_and_oversize_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("result.json");
        let golden = Manifest::decode(GOLDEN).unwrap();
        let stale = Manifest::new(
            golden.module_id().clone(),
            golden.repository(),
            golden.logs().to_vec(),
            vec![],
        )
        .unwrap()
        .to_canonical_bytes()
        .unwrap();
        let tampered = std::str::from_utf8(GOLDEN)
            .unwrap()
            .replacen(",\"name\":\"path\"", ",\"name\":\"work_path\"", 1)
            .into_bytes();
        let mut oversized = GOLDEN.to_vec();
        oversized.resize(MAX_OUTPUT_BYTES + 1, b' ');
        for (bytes, reason) in [
            (stale.as_slice(), "stale or not canonical"),
            (tampered.as_slice(), "templates_digest"),
            (&GOLDEN[..GOLDEN.len() - 1], "stale or not canonical"),
            (oversized.as_slice(), "too_large"),
        ] {
            fs::write(&out, bytes).unwrap();
            let stderr = assert_exit(
                command(&fixture(), Some(&out))
                    .arg("--check")
                    .output()
                    .unwrap(),
                1,
            );
            assert!(stderr.contains(reason), "{stderr}");
            assert_eq!(fs::read(&out).unwrap(), bytes);
        }
    }

    #[test]
    fn write_requires_metadata_and_preserves_output_on_invalid_or_oversize_result() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("result.json");
        let stderr = assert_exit(command(&fixture(), Some(&out)).output().unwrap(), 2);
        assert!(stderr.contains("creating a manifest requires"));
        assert!(!out.exists());

        fs::write(&out, GOLDEN).unwrap();
        assert_exit(
            command(&fixture(), Some(&out))
                .args(["--repository", "https://example.invalid"])
                .output()
                .unwrap(),
            1,
        );
        assert_eq!(fs::read(&out).unwrap(), GOLDEN);

        let source = tmp.path().join("src");
        fs::create_dir(&source).unwrap();
        fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = 'big-fixture'\nversion = '0.0.0'\nedition = '2021'\n[workspace]\n",
        )
        .unwrap();
        let mut rust = String::from("fn run() {\n");
        let filler = "z".repeat(1000);
        for n in 0..1100 {
            rust.push_str(&format!("info!(\"{n:04} {filler}\");\n"));
        }
        rust.push_str("}\n");
        fs::write(source.join("lib.rs"), rust).unwrap();
        let stderr = assert_exit(command(tmp.path(), Some(&out)).output().unwrap(), 1);
        assert!(stderr.contains("too_large"), "{stderr}");
        assert_eq!(fs::read(&out).unwrap(), GOLDEN);
    }

    #[test]
    fn autobins_false_keeps_explicit_bin_and_uses_default_output() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("src");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = 'manual-bins'\nversion = '0.0.0'\nedition = '2021'\nautobins = false\n[workspace]\n[[bin]]\nname = 'manual'\npath = 'manual.rs'\n").unwrap();
        fs::write(
            source.join("bin/excluded.rs"),
            "fn main() { info!(\"excluded flat bin\"); }",
        )
        .unwrap();
        fs::write(
            source.join("main.rs"),
            "fn main() { info!(\"excluded main\"); }",
        )
        .unwrap();
        fs::write(
            source.join("lib.rs"),
            "mod plain; fn run() { info!(\"included lib\"); }",
        )
        .unwrap();
        fs::write(
            tmp.path().join("manual.rs"),
            "fn main() { info!(\"included manual\"); }",
        )
        .unwrap();
        fs::write(
            source.join("plain.rs"),
            "#[path = \"selected\"] mod alternate { mod nested; }",
        )
        .unwrap();
        fs::create_dir(source.join("selected")).unwrap();
        fs::write(
            source.join("selected/nested.rs"),
            "fn run() { info!(\"selected child\"); }",
        )
        .unwrap();
        let metadata = ManifestMetadata::new(
            ModuleId::new("fixture").unwrap(),
            "https://github.com/example/fixture",
            vec![LogSource::new("logs/*.log").unwrap()],
        );
        let (manifest, skips) = generate(tmp.path(), &metadata).unwrap();
        assert!(skips.is_empty());
        let mut calls: Vec<_> = manifest
            .templates()
            .iter()
            .map(|t| (t.target(), t.text()))
            .collect();
        calls.sort_unstable();
        assert_eq!(
            calls,
            [
                ("manual", "included manual"),
                ("manual_bins", "included lib"),
                ("manual_bins::plain::alternate::nested", "selected child"),
            ]
        );

        assert_exit(
            metadata_flags(&mut command(tmp.path(), None))
                .output()
                .unwrap(),
            0,
        );
        let out = tmp.path().join("doctor-manifest.json");
        assert_eq!(
            fs::read(&out).unwrap(),
            manifest.to_canonical_bytes().unwrap()
        );
        assert_exit(
            command(tmp.path(), None)
                .args(["--log-glob", "old/*.log", "--log-glob", "new/*.log"])
                .output()
                .unwrap(),
            0,
        );
        let replaced = Manifest::decode(&fs::read(out).unwrap()).unwrap();
        assert_eq!(
            replaced
                .logs()
                .iter()
                .map(LogSource::glob)
                .collect::<Vec<_>>(),
            ["old/*.log", "new/*.log"]
        );
        assert_eq!(replaced.templates(), manifest.templates());
        assert_eq!(replaced.module_id(), manifest.module_id());
    }
}
