//! Explicitly ignored, real operator-verifier proof. No component installation.
use super::*;
const PIN: &str = "f42a35577c824eff67254f640386b7d6b686297ee2213b6e088758d274db8c58";
const BUNDLE: &str = "20291e9dbd552a2666b0fb191e1cc20c95392413e800055eca10a17f36c96a14";
const VERIFIER: &str = "ae1ecd212663f3693ad9edf8b1a183900c9a52d3155ba6e354237f9a0f6463fc";
fn record(root: &Path, name: &str, value: &serde_json::Value) {
    let path = root.join(format!("{name}.json"));
    let mut file = new_file(&path).unwrap();
    file.write_all(&serde_json::to_vec_pretty(value).unwrap()).unwrap();
    file.sync_all().unwrap();
    sync_dir(root).unwrap();
}
fn finish_case(root: &Path, name: &str, elapsed: Duration, result: &Result<()>, bytes: &[u8], verifier_expected: bool) {
    assert!(bytes.len() <= 65536);
    let mut file = new_file(&root.join(format!("{name}.diagnostic.bin"))).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    record(root, name, &serde_json::json!({
        "case":name,"elapsed_seconds":elapsed.as_secs_f64(),"result":result,
        "diagnostic_sha256":digest(bytes),"diagnostic_bytes":bytes.len(),
        "verifier_expected":verifier_expected,"cryptographic_reason_review_pending":verifier_expected && result.is_err(),
        "scope":"same-invocation bounded diagnostic; not automatic reason acceptance"
    }));
}
async fn aggregate_case(root: &Path, name: &str, body: &[u8], bundle: Option<&[u8]>, expected: &str, error: Option<&str>) {
    let case = root.join(name);
    private_dir(&case).unwrap();
    let pin = case.join("pinsets/sha256").join(digest(body));
    private_dir(&pin).unwrap();
    fs::write(pin.join("manifest.json"), body).unwrap();
    if let Some(bytes) = bundle { fs::write(pin.join("manifest.bundle"), bytes).unwrap(); }
    let policy = Policy::new(
        Some(format!("file://{}/manifest.json", pin.display())),
        Some(expected.into()), Some(format!("file://{}", case.display())),
    ).unwrap();
    let start = Instant::now();
    process::begin_diagnostic();
    let outcome = Aggregate::load(&policy, &case.join("evidence"), "unused").await;
    let diagnostic = process::take_diagnostic();
    let simple = outcome.as_ref().map(|_| ()).map_err(Clone::clone);
    finish_case(root, name, start.elapsed(), &simple, &diagnostic, !matches!(name, "wrong-pin" | "missing-bundle"));
    if let Some(expected_error) = error {
        assert_eq!(simple.unwrap_err(), fail(expected_error));
    } else {
        let aggregate = outcome.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(aggregate.sha, PIN);
        assert_eq!(aggregate.bundle_sha, BUNDLE);
        assert_eq!(aggregate.verifier_sha, VERIFIER);
        assert!(aggregate.pinned);
        for (name, sha, size, archive) in [
            ("enscrive-developer", "140e76334a71de986c9f01a03aabe44fbae8f6fce7811fdb9eba86eee07cd4e9", 28095388, true),
            ("enscrive-embed", "0ee7dc7c3ba010bdc4eb3891fc46efce2eedc94973000658f1de5386d0ed8440", 196905744, false),
        ] {
            let selection = aggregate.select(name, "x86_64-unknown-linux-gnu").unwrap();
            assert_eq!(selection.sha, sha);
            assert_eq!(selection.size, Some(size));
            assert_eq!(selection.archive, archive);
        }
    }
    assert!(!case.join("generations").exists());
    assert!(!case.join("profile.json").exists());
}
// This single selected current-thread test must serialize process environment across awaits.
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires separately approved operator verifier, exact public inputs, resource and network gate"]
async fn real_cosign_aggregate_and_expected_signer_proof() {
    let _lock = crate::test_support::lock_env();
    let inputs = PathBuf::from(std::env::var_os("ENS5913_REAL_INPUTS").expect("explicit input directory"));
    let root = PathBuf::from(std::env::var_os("ENS5913_REAL_OUTPUT").expect("explicit output directory"));
    assert!(inputs.is_absolute() && root.is_absolute());
    private_dir(&root).unwrap();
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    let body = fs::read(inputs.join("manifest.json")).unwrap();
    let bundle = fs::read(inputs.join("manifest.bundle")).unwrap();
    assert_eq!(digest(&body), PIN);
    assert_eq!(digest(&bundle), BUNDLE);
    let verifier = process::Verifier::resolve().unwrap();
    assert_eq!(verifier.digest(), VERIFIER);
    assert_eq!(IDENTITY, "https://github.com/enscrive/enscrive-cli/.github/workflows/manifest.yml@refs/heads/main");
    assert_eq!(ISSUER, "https://token.actions.githubusercontent.com");
    let start = Instant::now();
    record(&root, "inputs", &serde_json::json!({"manifest":PIN,"bundle":BUNDLE,"verifier":VERIFIER,"identity":IDENTITY,"issuer":ISSUER}));
    aggregate_case(&root, "positive-first", &body, Some(&bundle), PIN, None).await;
    aggregate_case(&root, "wrong-pin", &body, Some(&bundle), &"0".repeat(64), Some("manifest digest mismatch")).await;
    aggregate_case(&root, "missing-bundle", &body, None, PIN, Some("artifact file unavailable")).await;
    let mut changed = body.clone(); changed.push(b' ');
    aggregate_case(&root, "changed-manifest", &changed, Some(&bundle), &digest(&changed), Some("signature rejected")).await;
    aggregate_case(&root, "corrupt-bundle", &body, Some(&bundle[..bundle.len()/2]), PIN, Some("signature rejected")).await;
    for (name, identity, issuer) in [
        ("wrong-identity", "https://github.com/enscrive/enscrive-cli/.github/workflows/not-manifest.yml@refs/heads/main", ISSUER),
        ("wrong-issuer", IDENTITY, "https://not-the-issuer.invalid"),
    ] {
        let case = root.join(name); private_dir(&case).unwrap();
        let manifest = case.join("manifest.json"); let detached = case.join("manifest.bundle");
        fs::write(&manifest, &body).unwrap(); fs::write(&detached, &bundle).unwrap();
        let before_m = identity_for_test(&manifest); let before_b = identity_for_test(&detached);
        process::begin_diagnostic();
        let began = Instant::now();
        let result = verifier.verify_test_expectations(&manifest, &detached, &case, identity, issuer);
        let diagnostic = process::take_diagnostic();
        finish_case(&root, name, began.elapsed(), &result, &diagnostic, true);
        record(&root, &format!("{name}-expectations"), &serde_json::json!({"identity":identity,"issuer":issuer,"aggregate_path":false}));
        assert_eq!(result.unwrap_err(), fail("signature rejected"));
        assert!(before_m == identity_for_test(&manifest) && before_b == identity_for_test(&detached));
    }
    aggregate_case(&root, "positive-last", &body, Some(&bundle), PIN, None).await;
    record(&root, "matrix", &serde_json::json!({"cases":8,"elapsed_seconds":start.elapsed().as_secs_f64(),"execution_assertions_passed":true,"cryptographic_proof_pass":false,"reason_review_required":true,"component_installation":false}));
}
fn identity_for_test(path: &Path) -> Identity {
    identity(path, Instant::now() + Duration::from_secs(30), 4 * 1024 * 1024).unwrap()
}
