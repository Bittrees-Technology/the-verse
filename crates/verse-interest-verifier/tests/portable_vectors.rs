// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;
use verse_interest_verifier::{ErrorCode, InterestVerifier, StageKind, VerifierConfig};
use verse_protocol::SessionRole;

const DOMAIN: &[u8] = b"the-verse/interest-view/v1\0";
const REGISTRY_HASH: &str = "f00517b0fbef09d7924fde2cb11f2c74066627992ab900a6a9e0bd3ac3dc7311";
const MANIFEST_HASH: &str = "a3d5eb718f859d6010854f231a0e2cb4518c9618580020762311b4c3e43e3e06";
const CONTENT_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASELINE_HASH: &str = "05445f8eda0373f1f214b661584eceaa84f845deb02af757bfadb266b63ea2a2";
const DELTA_HASH: &str = "4cb9ff8804f86af5387c21e9e81511445b3364069b8d1ac5a1c2dba11cbbb5e5";

fn payload(bytes: &'static [u8]) -> &'static [u8] {
    bytes
        .strip_suffix(b"\n")
        .expect("published vector files have one terminating LF")
}

fn verifier() -> InterestVerifier {
    InterestVerifier::new(VerifierConfig::new(
        SessionRole::Player {
            player_id: "player-vector".to_owned(),
        },
        20,
        16,
        11,
        "p1.5.0",
        CONTENT_HASH,
        "universe-vector",
        REGISTRY_HASH,
        MANIFEST_HASH,
    ))
    .expect("portable-vector configuration is valid")
}

fn independent_digest(canonical: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN);
    hasher.update(canonical);
    hasher.finalize().to_hex().to_string()
}

fn independent_domain_digest(domain: &[u8], canonical: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(canonical);
    hasher.finalize().to_hex().to_string()
}

fn fixed_float(value: &mut serde_json::Value) {
    let scaled = (value.as_f64().expect("fixture float") * 1_000_000.0).round() as i64;
    *value = serde_json::json!(["fixed_1e6", scaled]);
}

fn fixed_vec(value: &mut serde_json::Value, axes: &[&str]) {
    for axis in axes {
        fixed_float(&mut value[*axis]);
    }
}

fn fixed_environment(value: &mut serde_json::Value) {
    fixed_vec(&mut value["planet_center"], &["x", "y", "z"]);
    for key in [
        "surface_radius_m",
        "distance_to_center_m",
        "distance_to_surface_m",
        "altitude_m",
        "gravity_m_s2",
        "atmosphere_density",
        "oxygen_fraction",
    ] {
        fixed_float(&mut value[key]);
    }
    fixed_vec(&mut value["gravity"], &["x", "y", "z"]);
}

fn fixed_public_player(value: &mut serde_json::Value) {
    fixed_vec(&mut value["orientation"], &["x", "y", "z", "w"]);
    fixed_vec(&mut value["linear_velocity"], &["x", "y", "z"]);
    fixed_vec(&mut value["angular_velocity"], &["x", "y", "z"]);
}

fn fixed_actor_private(value: &mut serde_json::Value) {
    let player = &mut value["player"];
    fixed_public_player(player);
    fixed_vec(&mut player["locomotion"]["up"], &["x", "y", "z"]);
    fixed_float(&mut player["locomotion"]["view_pitch_radians"]);
    fixed_vec(&mut player["control_linear_input"], &["x", "y", "z"]);
    fixed_vec(&mut player["control_angular_input"], &["x", "y", "z"]);
}

fn baseline_hash_material(frame: &serde_json::Value) -> serde_json::Value {
    let baseline = &frame["baseline"];
    let interest = &baseline["interest"];
    let mut entities = interest["entered"].clone();
    for entity in entities.as_array_mut().expect("entity vector") {
        if entity["payload"]["entity_kind"] == "player" {
            fixed_public_player(&mut entity["payload"]["value"]);
        }
    }
    let mut environment = baseline["environment"].clone();
    fixed_environment(&mut environment);
    let mut actor_private = baseline["actor_private"].clone();
    fixed_actor_private(&mut actor_private);
    serde_json::json!({
        "projection_schema_version": baseline["projection_schema_version"],
        "interest_schema_version": interest["schema_version"],
        "content_manifest_version": baseline["content_manifest_version"],
        "universe_id": baseline["universe_id"],
        "cell_id": baseline["cell_id"],
        "universe_manifest_hash": baseline["universe_manifest_hash"],
        "celestial_registry_hash": baseline["celestial_registry_hash"],
        "cell_address": baseline["cell_address"],
        "local_origin": interest["local_origin_address"],
        "gravity_body_id": baseline["gravity_body_id"],
        "voxel_body_id": baseline["voxel_body_id"],
        "observer_class": interest["observer_class"],
        "session_epoch": interest["session_epoch"],
        "interest_epoch": interest["interest_epoch"],
        "baseline_id": interest["baseline_id"],
        "delta_sequence": interest["delta_sequence"],
        "entities": entities,
        "environment": environment,
        "conservation_valid": baseline["conservation_valid"],
        "actor_private": actor_private,
    })
}

fn delta_hash_material(
    baseline_material: &serde_json::Value,
    delta_frame: &serde_json::Value,
) -> serde_json::Value {
    let mut material = baseline_material.clone();
    material["delta_sequence"] = delta_frame["delta"]["interest"]["delta_sequence"].clone();
    material["local_origin"] = delta_frame["delta"]["interest"]["local_origin_address"].clone();
    material
}

fn document_commitments(frame: &serde_json::Value) -> (String, String) {
    let mut registry = frame["registry"].clone();
    registry
        .as_object_mut()
        .expect("registry object")
        .remove("registry_hash");
    let mut manifest = frame["universe_manifest"].clone();
    manifest
        .as_object_mut()
        .expect("manifest object")
        .remove("manifest_hash");
    let registry_bytes = serde_json::to_vec(&registry).expect("registry canonical integers");
    let registry_hash =
        independent_domain_digest(b"the-verse/celestial-registry/v1\0", &registry_bytes);
    manifest["celestial_registry_hash"] = serde_json::Value::String(registry_hash.clone());
    let manifest_bytes = serde_json::to_vec(&manifest).expect("manifest canonical integers");
    let manifest_hash =
        independent_domain_digest(b"the-verse/universe-manifest/v4\0", &manifest_bytes);
    (registry_hash, manifest_hash)
}

fn independent_document_commitments() -> (String, String) {
    let frame: serde_json::Value =
        serde_json::from_slice(payload(include_bytes!("../test-vectors/v1/registry.json")))
            .expect("registry frame is JSON");
    document_commitments(&frame)
}

fn stage_commit(
    verifier: &mut InterestVerifier,
    raw: &'static [u8],
    expected_kind: StageKind,
    expected_ack: Option<&'static [u8]>,
) {
    let raw = payload(raw);
    let token = verifier.stage(raw).expect("published frame stages");
    assert_eq!(verifier.pending_kind(), Some(expected_kind));
    assert_eq!(
        verifier.pending_sanitized_json(),
        Some(std::str::from_utf8(raw).unwrap())
    );
    let outcome = verifier.commit(token).expect("published frame commits");
    assert_eq!(outcome.kind, expected_kind);
    assert_eq!(
        outcome.acknowledgement_json.as_deref().map(str::as_bytes),
        expected_ack.map(payload)
    );
}

#[derive(Clone, Copy)]
enum VectorTarget {
    Registry,
    Baseline,
    Delta,
}

impl VectorTarget {
    fn raw(self) -> &'static [u8] {
        match self {
            Self::Registry => include_bytes!("../test-vectors/v1/registry.json"),
            Self::Baseline => include_bytes!("../test-vectors/v1/baseline.json"),
            Self::Delta => include_bytes!("../test-vectors/v1/delta.json"),
        }
    }

    const fn acknowledgement_expected(self) -> bool {
        !matches!(self, Self::Registry)
    }
}

struct InvalidCase {
    name: &'static str,
    target: VectorTarget,
    expected: ErrorCode,
    mutate: fn(&mut serde_json::Value),
}

fn invalidate_registry_hash(frame: &mut serde_json::Value) {
    let wrong = serde_json::Value::String("0".repeat(64));
    frame["registry"]["registry_hash"] = wrong.clone();
    frame["universe_manifest"]["celestial_registry_hash"] = wrong;
}

fn invalidate_manifest_hash(frame: &mut serde_json::Value) {
    frame["universe_manifest"]["manifest_hash"] = serde_json::Value::String("0".repeat(64));
}

fn make_address_noncanonical(frame: &mut serde_json::Value) {
    frame["baseline"]["cell_address"]["sector"]["x"] = serde_json::json!("-0");
}

fn invalidate_body_reference(frame: &mut serde_json::Value) {
    frame["baseline"]["gravity_body_id"] = serde_json::json!("body-absent");
}

fn invalidate_private_linkage(frame: &mut serde_json::Value) {
    frame["baseline"]["actor_private"]["player"]["inventory_id"] =
        serde_json::json!("inventory-absent");
}

fn invalidate_view_hash(frame: &mut serde_json::Value) {
    frame["baseline"]["interest"]["view_hash"] = serde_json::Value::String("0".repeat(64));
}

fn invalidate_delta_frontier(frame: &mut serde_json::Value) {
    frame["delta"]["interest"]["delta_sequence"] = serde_json::json!(2);
}

fn commit_prerequisites(verifier: &mut InterestVerifier, target: VectorTarget) {
    let prerequisites: &[&[u8]] = match target {
        VectorTarget::Registry => &[include_bytes!("../test-vectors/v1/welcome.json")],
        VectorTarget::Baseline => &[
            include_bytes!("../test-vectors/v1/welcome.json"),
            include_bytes!("../test-vectors/v1/registry.json"),
        ],
        VectorTarget::Delta => &[
            include_bytes!("../test-vectors/v1/welcome.json"),
            include_bytes!("../test-vectors/v1/registry.json"),
            include_bytes!("../test-vectors/v1/baseline.json"),
        ],
    };
    for raw in prerequisites {
        let token = verifier
            .stage(payload(raw))
            .expect("published prerequisite stages");
        verifier
            .commit(token)
            .expect("published prerequisite commits");
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvalidCorpus {
    schema_version: u32,
    corpus: String,
    license: String,
    encoding: String,
    cases: Vec<RawInvalidCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInvalidCase {
    name: String,
    frame: String,
    target: String,
    prerequisites: Vec<String>,
    expected_code: String,
    recovery_frame: String,
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/v1")
        .join(name);
    let mut raw = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("read frozen fixture {}: {error}", path.display()));
    assert!(raw.ends_with(b"\n"), "{} terminates in LF", path.display());
    assert!(
        !raw.ends_with(b"\n\n"),
        "{} terminates in exactly one LF",
        path.display()
    );
    raw.pop();
    raw
}

fn commit_fixture(verifier: &mut InterestVerifier, name: &str) {
    let raw = fixture_bytes(name);
    let token = verifier
        .stage(&raw)
        .unwrap_or_else(|error| panic!("prerequisite {name} stages: {error}"));
    verifier
        .commit(token)
        .unwrap_or_else(|error| panic!("prerequisite {name} commits: {error}"));
}

#[test]
fn published_v1_vectors_verify_commit_and_ack_exactly() {
    let manifest: serde_json::Value =
        serde_json::from_slice(payload(include_bytes!("../test-vectors/v1/manifest.json")))
            .expect("vector manifest is JSON");
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["license"]["spdx"], "Apache-2.0");
    assert_eq!(
        manifest["domain"]["hex"],
        "7468652d76657273652f696e7465726573742d766965772f763100"
    );
    assert_eq!(
        manifest["cases"]["baseline"]["expected_view_hash"],
        BASELINE_HASH
    );
    assert_eq!(manifest["cases"]["delta"]["expected_view_hash"], DELTA_HASH);
    assert_eq!(
        manifest["verifier_config"]["expected_role"]["player_id"],
        "player-vector"
    );
    assert_eq!(manifest["verifier_config"]["world_schema_version"], 20);
    assert_eq!(manifest["verifier_config"]["event_schema_version"], 16);
    assert_eq!(manifest["verifier_config"]["content_schema_version"], 11);
    assert_eq!(
        manifest["verifier_config"]["content_manifest_version"],
        "p1.5.0"
    );
    assert_eq!(
        manifest["verifier_config"]["expected_content_hash"],
        CONTENT_HASH
    );
    assert_eq!(
        manifest["verifier_config"]["expected_universe_id"],
        "universe-vector"
    );
    assert_eq!(
        manifest["verifier_config"]["expected_celestial_registry_hash"],
        REGISTRY_HASH
    );
    assert_eq!(
        manifest["verifier_config"]["expected_universe_manifest_hash"],
        MANIFEST_HASH
    );
    assert_eq!(
        manifest["document_commitments"]["celestial_registry_hash"],
        REGISTRY_HASH
    );
    assert_eq!(
        manifest["document_commitments"]["universe_manifest_hash"],
        MANIFEST_HASH
    );
    assert_eq!(
        independent_document_commitments(),
        (REGISTRY_HASH.to_owned(), MANIFEST_HASH.to_owned())
    );

    let baseline_frame: serde_json::Value =
        serde_json::from_slice(payload(include_bytes!("../test-vectors/v1/baseline.json")))
            .expect("baseline frame is JSON");
    let delta_frame: serde_json::Value =
        serde_json::from_slice(payload(include_bytes!("../test-vectors/v1/delta.json")))
            .expect("delta frame is JSON");
    let baseline_material = baseline_hash_material(&baseline_frame);
    let delta_material = delta_hash_material(&baseline_material, &delta_frame);
    assert_eq!(
        serde_json::to_vec(&baseline_material).expect("baseline canonical material"),
        payload(include_bytes!("../test-vectors/v1/baseline.canonical.json"))
    );
    assert_eq!(
        serde_json::to_vec(&delta_material).expect("delta canonical material"),
        payload(include_bytes!("../test-vectors/v1/delta.canonical.json"))
    );

    assert_eq!(
        independent_digest(payload(include_bytes!(
            "../test-vectors/v1/baseline.canonical.json"
        ))),
        BASELINE_HASH
    );
    assert_eq!(
        independent_digest(payload(include_bytes!(
            "../test-vectors/v1/delta.canonical.json"
        ))),
        DELTA_HASH
    );

    let mut verifier = verifier();
    stage_commit(
        &mut verifier,
        include_bytes!("../test-vectors/v1/welcome.json"),
        StageKind::Welcome,
        None,
    );
    stage_commit(
        &mut verifier,
        include_bytes!("../test-vectors/v1/registry.json"),
        StageKind::Registry,
        None,
    );
    stage_commit(
        &mut verifier,
        include_bytes!("../test-vectors/v1/baseline.json"),
        StageKind::Baseline,
        Some(include_bytes!("../test-vectors/v1/baseline.ack.json")),
    );
    let baseline = verifier.committed_view().expect("baseline commits a view");
    assert_eq!(baseline.interest_epoch, 9_007_199_254_740_993);
    assert_eq!(baseline.delta_sequence, 0);
    assert_eq!(baseline.view_hash, BASELINE_HASH);
    assert_eq!(baseline.entity_count, 1);
    assert!(baseline.has_actor_private);

    stage_commit(
        &mut verifier,
        include_bytes!("../test-vectors/v1/delta.json"),
        StageKind::Delta,
        Some(include_bytes!("../test-vectors/v1/delta.ack.json")),
    );
    let delta = verifier.committed_view().expect("delta commits a view");
    assert_eq!(delta.interest_epoch, 9_007_199_254_740_993);
    assert_eq!(delta.delta_sequence, 1);
    assert_eq!(delta.view_hash, DELTA_HASH);
    assert_eq!(delta.entity_count, 1);
    assert!(delta.has_actor_private);
}

#[test]
fn invalid_vector_table_fails_closed_without_state_or_acknowledgement() {
    let cases = [
        InvalidCase {
            name: "registry_hash",
            target: VectorTarget::Registry,
            expected: ErrorCode::BindingMismatch,
            mutate: invalidate_registry_hash,
        },
        InvalidCase {
            name: "manifest_hash",
            target: VectorTarget::Registry,
            expected: ErrorCode::BindingMismatch,
            mutate: invalidate_manifest_hash,
        },
        InvalidCase {
            name: "noncanonical_address",
            target: VectorTarget::Baseline,
            expected: ErrorCode::InvalidAddress,
            mutate: make_address_noncanonical,
        },
        InvalidCase {
            name: "unknown_body_reference",
            target: VectorTarget::Baseline,
            expected: ErrorCode::BindingMismatch,
            mutate: invalidate_body_reference,
        },
        InvalidCase {
            name: "private_linkage",
            target: VectorTarget::Baseline,
            expected: ErrorCode::InvalidPrivateLinkage,
            mutate: invalidate_private_linkage,
        },
        InvalidCase {
            name: "view_hash",
            target: VectorTarget::Baseline,
            expected: ErrorCode::HashMismatch,
            mutate: invalidate_view_hash,
        },
        InvalidCase {
            name: "delta_frontier",
            target: VectorTarget::Delta,
            expected: ErrorCode::FrontierMismatch,
            mutate: invalidate_delta_frontier,
        },
    ];
    let manifest: serde_json::Value =
        serde_json::from_slice(payload(include_bytes!("../test-vectors/v1/manifest.json")))
            .expect("vector manifest is JSON");

    for case in cases {
        assert_eq!(
            manifest["invalid_cases"][case.name],
            case.expected.as_str(),
            "manifest error code for {}",
            case.name
        );
        let mut verifier = verifier();
        commit_prerequisites(&mut verifier, case.target);
        let before = verifier.committed_view();
        let mut invalid: serde_json::Value =
            serde_json::from_slice(payload(case.target.raw())).expect("published target is JSON");
        (case.mutate)(&mut invalid);
        let raw = serde_json::to_vec(&invalid).expect("invalid vector serializes");
        let error = verifier
            .stage(&raw)
            .expect_err("invalid vector must not produce a stage token or ACK");
        assert_eq!(error.code(), case.expected, "invalid case {}", case.name);
        assert_eq!(verifier.pending_kind(), None, "invalid case {}", case.name);
        assert_eq!(
            verifier.pending_sanitized_json(),
            None,
            "invalid case {}",
            case.name
        );
        assert_eq!(
            verifier.committed_view(),
            before,
            "invalid case {}",
            case.name
        );

        let recovery = verifier
            .stage(payload(case.target.raw()))
            .expect("the exact published frame still stages after rejection");
        let outcome = verifier
            .commit(recovery)
            .expect("the exact published frame still commits after rejection");
        assert_eq!(
            outcome.acknowledgement_json.is_some(),
            case.target.acknowledgement_expected(),
            "recovery ACK shape for {}",
            case.name
        );
    }
}

#[test]
fn frozen_raw_invalid_corpus_is_exhaustive_fail_closed_and_recoverable() {
    let corpus: InvalidCorpus =
        serde_json::from_slice(&fixture_bytes("invalid-corpus.json")).expect("corpus manifest");
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.corpus, "the-verse-interest-invalid-v1");
    assert_eq!(corpus.license, "Apache-2.0");
    assert!(corpus.encoding.contains("raw compact UTF-8 JSON"));
    assert_eq!(corpus.cases.len(), 16, "the bounded corpus size is frozen");

    let declared: std::collections::BTreeSet<_> =
        corpus.cases.iter().map(|case| case.frame.clone()).collect();
    let invalid_directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/v1/invalid");
    let published: std::collections::BTreeSet<_> = std::fs::read_dir(invalid_directory)
        .expect("read invalid fixture directory")
        .map(|entry| {
            format!(
                "invalid/{}",
                entry
                    .expect("read invalid fixture entry")
                    .file_name()
                    .to_string_lossy()
            )
        })
        .collect();
    assert_eq!(
        declared, published,
        "manifest lists every raw invalid frame"
    );

    for case in &corpus.cases {
        let mut verifier = verifier();
        for prerequisite in &case.prerequisites {
            commit_fixture(&mut verifier, prerequisite);
        }
        let before = verifier.committed_view();
        let invalid = fixture_bytes(&case.frame);
        let error = verifier
            .stage(&invalid)
            .expect_err("frozen invalid frame must not produce a stage token");
        assert_eq!(
            error.code().as_str(),
            case.expected_code,
            "stable code for {}",
            case.name
        );
        assert_eq!(
            verifier.pending_kind(),
            None,
            "pending kind for {}",
            case.name
        );
        assert_eq!(
            verifier.pending_sanitized_json(),
            None,
            "pending bytes for {}",
            case.name
        );
        assert_eq!(
            verifier.committed_view(),
            before,
            "committed state for {}",
            case.name
        );

        let recovery = fixture_bytes(&case.recovery_frame);
        let token = verifier
            .stage(&recovery)
            .unwrap_or_else(|error| panic!("{} recovery stages: {error}", case.name));
        let outcome = verifier
            .commit(token)
            .unwrap_or_else(|error| panic!("{} recovery commits: {error}", case.name));
        let acknowledgement_expected = matches!(case.target.as_str(), "baseline" | "delta");
        assert_eq!(
            outcome.acknowledgement_json.is_some(),
            acknowledgement_expected,
            "recovery ACK shape for {}",
            case.name
        );
    }

    let substituted: serde_json::Value =
        serde_json::from_slice(&fixture_bytes("invalid/substituted-roots.json"))
            .expect("substituted roots are well-formed JSON");
    let (registry_hash, manifest_hash) = document_commitments(&substituted);
    assert_eq!(substituted["registry"]["registry_hash"], registry_hash);
    assert_eq!(
        substituted["universe_manifest"]["celestial_registry_hash"],
        registry_hash
    );
    assert_eq!(
        substituted["universe_manifest"]["manifest_hash"],
        manifest_hash
    );
    assert_ne!(registry_hash, REGISTRY_HASH, "registry root is substituted");
    assert_ne!(manifest_hash, MANIFEST_HASH, "manifest root is substituted");
}

#[test]
fn tampered_raw_delta_is_rejected_without_commit_or_ack() {
    let mut verifier = verifier();
    for frame in [
        include_bytes!("../test-vectors/v1/welcome.json").as_slice(),
        include_bytes!("../test-vectors/v1/registry.json").as_slice(),
        include_bytes!("../test-vectors/v1/baseline.json").as_slice(),
    ] {
        let token = verifier
            .stage(payload(frame))
            .expect("prerequisite frame stages");
        verifier.commit(token).expect("prerequisite frame commits");
    }
    let before = verifier.committed_view().expect("baseline view exists");
    let raw_delta = std::str::from_utf8(payload(include_bytes!("../test-vectors/v1/delta.json")))
        .expect("delta is UTF-8");
    let tampered = raw_delta.replacen(
        "\"local_origin_address\":{\"universe_id\":\"universe-vector\",\"sector\":{\"x\":\"0\",\"y\":\"0\",\"z\":\"0\"},\"cell\":{\"x\":1,\"y\":2,\"z\":3},\"local_um\":{\"x\":-50,\"y\":49,\"z\":0}}",
        "\"local_origin_address\":{\"universe_id\":\"universe-vector\",\"sector\":{\"x\":\"0\",\"y\":\"0\",\"z\":\"0\"},\"cell\":{\"x\":1,\"y\":2,\"z\":3},\"local_um\":{\"x\":-49,\"y\":49,\"z\":0}}",
        1,
    );
    assert_ne!(tampered, raw_delta, "tamper target must be present");
    let error = verifier
        .stage(tampered.as_bytes())
        .expect_err("changed hash material is rejected");
    assert_eq!(error.code(), ErrorCode::HashMismatch);
    assert_eq!(verifier.pending_kind(), None);
    assert_eq!(verifier.committed_view(), Some(before));
}
