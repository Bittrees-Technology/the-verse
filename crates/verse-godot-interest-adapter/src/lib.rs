// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fail-closed Godot bridge for the portable interest-view verifier.

#![deny(unsafe_code)]

use godot::classes::{IRefCounted, RefCounted};
use godot::prelude::*;
use serde_json::Value;
use verse_interest_verifier::{
    InterestVerifier, StageKind, StageToken, VerifierConfig, VerifyError,
};
use verse_protocol::SessionRole;

#[derive(Debug)]
struct PendingBridgeStage {
    bridge_token: u64,
    core_token: StageToken,
}

#[derive(Debug, Default)]
struct AdapterSession {
    verifier: Option<InterestVerifier>,
    pending: Option<PendingBridgeStage>,
    next_token: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct StagedFrame {
    token: u64,
    kind: &'static str,
    sanitized_json: String,
}

const JSON_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;
const LOSSLESS_INTEGER_PREFIX: &str = "__VERSE_LOSSLESS_INTEGER__:";
const LOSSLESS_STRING_PREFIX: &str = "__VERSE_LOSSLESS_STRING__:";

#[derive(Debug, PartialEq, Eq)]
struct CommittedFrame {
    kind: &'static str,
    acknowledgement: Option<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
struct BridgeError {
    code: &'static str,
    detail: String,
}

impl BridgeError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl From<VerifyError> for BridgeError {
    fn from(error: VerifyError) -> Self {
        Self::new(error.code().as_str(), error.detail())
    }
}

impl AdapterSession {
    #[allow(clippy::too_many_arguments)] // Complete pinned connection contract crosses this boundary.
    fn reset_player(
        &mut self,
        player_id: &str,
        world_schema_version: u32,
        event_schema_version: u32,
        content_schema_version: u32,
        content_manifest_version: &str,
        expected_content_hash: &str,
        expected_universe_id: &str,
        expected_celestial_registry_hash: &str,
        expected_universe_manifest_hash: &str,
    ) -> Result<(), BridgeError> {
        let config = VerifierConfig::new(
            SessionRole::Player {
                player_id: player_id.to_owned(),
            },
            world_schema_version,
            event_schema_version,
            content_schema_version,
            content_manifest_version,
            expected_content_hash,
            expected_universe_id,
            expected_celestial_registry_hash,
            expected_universe_manifest_hash,
        );
        self.verifier = Some(InterestVerifier::new(config).map_err(BridgeError::from)?);
        self.pending = None;
        self.next_token = next_bridge_token(self.next_token);
        Ok(())
    }

    fn stage(&mut self, raw: &[u8]) -> Result<StagedFrame, BridgeError> {
        if self.pending.is_some() {
            return Err(BridgeError::new(
                "pending_stage",
                "the Godot adapter already has a pending stage",
            ));
        }
        let verifier = self.verifier.as_mut().ok_or_else(|| {
            BridgeError::new(
                "unexpected_message",
                "the Godot adapter has not been configured for this connection",
            )
        })?;
        let core_token = verifier.stage(raw).map_err(BridgeError::from)?;
        let kind = verifier
            .pending_kind()
            .map(stage_kind_name)
            .expect("a successful core stage always has a pending kind");
        let sanitized_json = verifier
            .pending_sanitized_json()
            .expect("a successful core stage always has sanitized JSON")
            .to_owned();
        let sanitized_json = match presentation_safe_json(&sanitized_json) {
            Ok(json) => json,
            Err(error) => {
                verifier
                    .discard(core_token)
                    .expect("a successful stage has a discardable core token");
                return Err(error);
            }
        };
        self.next_token = next_bridge_token(self.next_token);
        let bridge_token = self.next_token;
        self.pending = Some(PendingBridgeStage {
            bridge_token,
            core_token,
        });
        Ok(StagedFrame {
            token: bridge_token,
            kind,
            sanitized_json,
        })
    }

    fn commit(&mut self, token: u64) -> Result<CommittedFrame, BridgeError> {
        let pending = self.take_matching_pending(token)?;
        let outcome = self
            .verifier
            .as_mut()
            .expect("pending adapter state requires a configured verifier")
            .commit(pending.core_token)
            .map_err(BridgeError::from)?;
        Ok(CommittedFrame {
            kind: stage_kind_name(outcome.kind),
            acknowledgement: outcome.acknowledgement_json.map(String::into_bytes),
        })
    }

    fn discard(&mut self, token: u64) -> Result<(), BridgeError> {
        let pending = self.take_matching_pending(token)?;
        self.verifier
            .as_mut()
            .expect("pending adapter state requires a configured verifier")
            .discard(pending.core_token)
            .map_err(BridgeError::from)
    }

    fn take_matching_pending(&mut self, token: u64) -> Result<PendingBridgeStage, BridgeError> {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.bridge_token != token)
        {
            return Err(BridgeError::new(
                "invalid_stage_token",
                "stage token is not current for the Godot connection",
            ));
        }
        Ok(self.pending.take().expect("pending token was checked"))
    }
}

fn presentation_safe_json(sanitized_json: &str) -> Result<String, BridgeError> {
    let mut value: Value = serde_json::from_str(sanitized_json).map_err(|error| {
        BridgeError::new(
            "invalid_presentation",
            format!("verified sanitized JSON could not be decoded: {error}"),
        )
    })?;
    encode_lossless_integers(&mut value);
    serde_json::to_string(&value).map_err(|error| {
        BridgeError::new(
            "invalid_presentation",
            format!("verified presentation JSON could not be encoded: {error}"),
        )
    })
}

fn encode_lossless_integers(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(encode_lossless_integers),
        Value::Object(values) => values.values_mut().for_each(encode_lossless_integers),
        Value::Number(number) => {
            let unsafe_integer = number
                .as_u64()
                .filter(|candidate| *candidate > JSON_SAFE_INTEGER_MAX)
                .map(|candidate| candidate.to_string())
                .or_else(|| {
                    number.as_i64().and_then(|candidate| {
                        candidate
                            .unsigned_abs()
                            .gt(&JSON_SAFE_INTEGER_MAX)
                            .then(|| candidate.to_string())
                    })
                });
            if let Some(decimal) = unsafe_integer {
                *value = Value::String(format!("{LOSSLESS_INTEGER_PREFIX}{decimal}"));
            }
        }
        Value::String(text) if text.starts_with("__VERSE_LOSSLESS_") => {
            *text = format!("{LOSSLESS_STRING_PREFIX}{text}");
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

const fn next_bridge_token(current: u64) -> u64 {
    if current >= i64::MAX as u64 {
        1
    } else {
        current + 1
    }
}

fn stage_kind_name(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Welcome => "welcome",
        StageKind::Registry => "registry",
        StageKind::Baseline => "baseline",
        StageKind::Delta => "delta",
        StageKind::IntentAccepted => "intent_accepted",
        StageKind::IntentRejected => "intent_rejected",
        StageKind::Fatal => "fatal",
    }
}

/// Connection-local native verifier exposed to `GDScript`.
#[derive(GodotClass)]
#[class(base = RefCounted)]
struct VerseInterestVerifier {
    session: AdapterSession,
    base: Base<RefCounted>,
}

#[godot_api]
impl IRefCounted for VerseInterestVerifier {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            session: AdapterSession::default(),
            base,
        }
    }
}

#[godot_api]
impl VerseInterestVerifier {
    #[func]
    #[allow(clippy::needless_pass_by_value)] // Godot's ABI owns argument values.
    #[allow(clippy::too_many_arguments)] // Complete pinned connection contract crosses this boundary.
    fn reset_player(
        &mut self,
        player_id: GString,
        world_schema_version: i64,
        event_schema_version: i64,
        content_schema_version: i64,
        content_manifest_version: GString,
        expected_content_hash: GString,
        expected_universe_id: GString,
        expected_celestial_registry_hash: GString,
        expected_universe_manifest_hash: GString,
    ) -> VarDictionary {
        let versions = (
            u32::try_from(world_schema_version),
            u32::try_from(event_schema_version),
            u32::try_from(content_schema_version),
        );
        let (Ok(world), Ok(event), Ok(content)) = versions else {
            return error_dictionary("incompatible_welcome", "schema version is outside u32");
        };
        match self.session.reset_player(
            &player_id.to_string(),
            world,
            event,
            content,
            &content_manifest_version.to_string(),
            &expected_content_hash.to_string(),
            &expected_universe_id.to_string(),
            &expected_celestial_registry_hash.to_string(),
            &expected_universe_manifest_hash.to_string(),
        ) {
            Ok(()) => ok_dictionary(),
            Err(error) => verifier_error_dictionary(&error),
        }
    }

    #[func]
    #[allow(clippy::needless_pass_by_value)] // Godot's ABI owns argument values.
    fn stage_server_message(&mut self, raw_utf8: PackedByteArray) -> VarDictionary {
        match self.session.stage(raw_utf8.as_slice()) {
            Ok(staged) => {
                let mut result = ok_dictionary();
                result.set(
                    "token",
                    i64::try_from(staged.token).expect("bridge tokens are bounded to i64"),
                );
                result.set("kind", staged.kind);
                result.set("sanitized_frame", staged.sanitized_json);
                result
            }
            Err(error) => verifier_error_dictionary(&error),
        }
    }

    #[func]
    fn commit(&mut self, token: i64) -> VarDictionary {
        let Ok(token) = u64::try_from(token) else {
            return error_dictionary("invalid_stage_token", "stage token is negative");
        };
        match self.session.commit(token) {
            Ok(committed) => {
                let mut result = ok_dictionary();
                result.set("kind", committed.kind);
                if let Some(acknowledgement) = committed.acknowledgement {
                    let acknowledgement = PackedByteArray::from(acknowledgement);
                    result.set("acknowledgement", &acknowledgement);
                }
                result
            }
            Err(error) => verifier_error_dictionary(&error),
        }
    }

    #[func]
    fn discard(&mut self, token: i64) -> VarDictionary {
        let Ok(token) = u64::try_from(token) else {
            return error_dictionary("invalid_stage_token", "stage token is negative");
        };
        match self.session.discard(token) {
            Ok(()) => ok_dictionary(),
            Err(error) => verifier_error_dictionary(&error),
        }
    }
}

fn ok_dictionary() -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("ok", true);
    result
}

fn verifier_error_dictionary(error: &BridgeError) -> VarDictionary {
    error_dictionary(error.code, &error.detail)
}

fn error_dictionary(code: &str, detail: &str) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("ok", false);
    result.set("error_code", code);
    result.set("detail", detail);
    result
}

// SAFETY: Godot's registration ABI requires this marker impl. It contains no
// unsafe block and all frame processing remains in safe Rust. The allowance is
// intentionally limited to this generated-ABI module.
#[allow(unsafe_code)]
mod extension_entry {
    use godot::prelude::{ExtensionLibrary, gdextension};

    #[allow(dead_code)] // Instantiated through the generated extension entry point.
    struct VerseGodotExtension;

    #[gdextension]
    unsafe impl ExtensionLibrary for VerseGodotExtension {}
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterSession, CommittedFrame, LOSSLESS_INTEGER_PREFIX, LOSSLESS_STRING_PREFIX,
        presentation_safe_json,
    };
    use verse_interest_verifier::{InterestVerifier, VerifierConfig};
    use verse_protocol::SessionRole;

    const WELCOME: &[u8] = br#"{"type":"welcome","protocol_version":18,"projection_schema_version":4,"world_schema_version":20,"event_schema_version":16,"content_schema_version":11,"content_manifest_version":"p1.5.0","celestial_registry_schema_version":1,"universe_manifest_schema_version":4,"interest_schema_version":2,"server_name":"adapter-test","session_role":{"kind":"player","player_id":"player-local"}}"#;
    const CONTENT_HASH: &str = "fc61c05b335fb951868010ecf2942a92ec4f03d00d0a75d3acba8c6f5162b6bd";
    const UNIVERSE_ID: &str = "the-verse-local";
    const REGISTRY_HASH: &str = "4c367bbfa04218ece14104f0a3a7ec2c7e9fefcc37d4cf78a265df2d711a59da";
    const MANIFEST_HASH: &str = "ce89422bd5d0c4a2ddc50f22883439a7ee1ecd7dd14165a46bb500623fd0b7eb";

    fn session() -> AdapterSession {
        let mut session = AdapterSession::default();
        session
            .reset_player(
                "player-local",
                20,
                16,
                11,
                "p1.5.0",
                CONTENT_HASH,
                UNIVERSE_ID,
                REGISTRY_HASH,
                MANIFEST_HASH,
            )
            .expect("valid adapter configuration");
        session
    }

    fn vector_payload(bytes: &'static [u8]) -> &'static [u8] {
        bytes
            .strip_suffix(b"\n")
            .expect("published vectors terminate with LF")
    }

    fn vector_session() -> AdapterSession {
        AdapterSession {
            verifier: Some(
                InterestVerifier::new(VerifierConfig::new(
                    SessionRole::Player {
                        player_id: "player-vector".to_owned(),
                    },
                    20,
                    16,
                    11,
                    "p1.5.0",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "universe-vector",
                    "f00517b0fbef09d7924fde2cb11f2c74066627992ab900a6a9e0bd3ac3dc7311",
                    "a3d5eb718f859d6010854f231a0e2cb4518c9618580020762311b4c3e43e3e06",
                ))
                .expect("vector verifier config"),
            ),
            pending: None,
            next_token: 0,
        }
    }

    fn commit_vector(session: &mut AdapterSession, raw: &'static [u8]) -> CommittedFrame {
        let staged = session
            .stage(vector_payload(raw))
            .expect("published vector stages");
        session
            .commit(staged.token)
            .expect("published vector commits")
    }

    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../verse-interest-verifier/test-vectors/v1")
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

    #[test]
    fn rejects_frames_until_connection_is_configured() {
        let error = AdapterSession::default()
            .stage(WELCOME)
            .expect_err("unconfigured adapter must reject frames");
        assert_eq!(error.code, "unexpected_message");
    }

    #[test]
    fn rejected_tamper_does_not_advance_handshake_state() {
        let mut session = session();
        let tampered = String::from_utf8(WELCOME.to_vec())
            .expect("fixture is UTF-8")
            .replace("\"world_schema_version\":20", "\"world_schema_version\":21");
        let error = session
            .stage(tampered.as_bytes())
            .expect_err("tampered tuple must be rejected");
        assert_eq!(error.code, "incompatible_welcome");

        let staged = session
            .stage(WELCOME)
            .expect("valid welcome must still stage after rejection");
        assert_eq!(staged.kind, "welcome");
        assert!(staged.sanitized_json.contains("\"protocol_version\":18"));
        let committed = session.commit(staged.token).expect("welcome commit");
        assert_eq!(committed.kind, "welcome");
        assert_eq!(committed.acknowledgement, None);
    }

    #[test]
    fn discard_and_reset_invalidate_connection_local_tokens() {
        let mut session = session();
        let discarded = session.stage(WELCOME).expect("welcome stage");
        session.discard(discarded.token).expect("discard token");
        let reused = session
            .commit(discarded.token)
            .expect_err("discarded token is invalid");
        assert_eq!(reused.code, "invalid_stage_token");

        let staged = session.stage(WELCOME).expect("restaged welcome");
        session
            .reset_player(
                "player-local",
                20,
                16,
                11,
                "p1.5.0",
                CONTENT_HASH,
                UNIVERSE_ID,
                REGISTRY_HASH,
                MANIFEST_HASH,
            )
            .expect("connection reset");
        let invalidated = session
            .commit(staged.token)
            .expect_err("reset token is invalid");
        assert_eq!(invalidated.code, "invalid_stage_token");
        assert!(session.stage(WELCOME).is_ok());
    }

    #[test]
    fn hash_tamper_never_commits_or_emits_an_acknowledgement() {
        const WELCOME_VECTOR: &[u8] =
            include_bytes!("../../verse-interest-verifier/test-vectors/v1/welcome.json");
        const REGISTRY_VECTOR: &[u8] =
            include_bytes!("../../verse-interest-verifier/test-vectors/v1/registry.json");
        const BASELINE_VECTOR: &[u8] =
            include_bytes!("../../verse-interest-verifier/test-vectors/v1/baseline.json");
        const BASELINE_ACK: &[u8] =
            include_bytes!("../../verse-interest-verifier/test-vectors/v1/baseline.ack.json");

        let mut session = vector_session();
        assert_eq!(
            commit_vector(&mut session, WELCOME_VECTOR).acknowledgement,
            None
        );
        assert_eq!(
            commit_vector(&mut session, REGISTRY_VECTOR).acknowledgement,
            None
        );

        let tampered = std::str::from_utf8(vector_payload(BASELINE_VECTOR))
            .expect("vector is UTF-8")
            .replace("\"altitude_m\":-5e-7", "\"altitude_m\":-1.5e-6");
        let error = session
            .stage(tampered.as_bytes())
            .expect_err("changed hash material must be rejected");
        assert_eq!(error.code, "hash_mismatch");
        assert!(session.pending.is_none());

        let committed = commit_vector(&mut session, BASELINE_VECTOR);
        let staged = session
            .stage(vector_payload(include_bytes!(
                "../../verse-interest-verifier/test-vectors/v1/delta.json"
            )))
            .expect("portable delta stages");
        assert!(
            staged
                .sanitized_json
                .contains(&format!("{LOSSLESS_INTEGER_PREFIX}9007199254740993")),
            "unsafe view frontiers must cross Godot JSON as lossless strings"
        );
        assert_eq!(
            committed.acknowledgement.as_deref(),
            Some(vector_payload(BASELINE_ACK))
        );
        let committed = session
            .commit(staged.token)
            .expect("portable delta commits");
        assert_eq!(
            committed.acknowledgement.as_deref(),
            Some(vector_payload(include_bytes!(
                "../../verse-interest-verifier/test-vectors/v1/delta.ack.json"
            )))
        );
    }

    #[test]
    fn frozen_raw_invalid_corpus_is_fail_closed_and_recoverable_through_godot_adapter() {
        let corpus: serde_json::Value =
            serde_json::from_slice(&fixture_bytes("invalid-corpus.json"))
                .expect("frozen invalid corpus manifest");
        assert_eq!(corpus["schema_version"], 1);
        assert_eq!(corpus["corpus"], "the-verse-interest-invalid-v1");
        assert_eq!(corpus["license"], "Apache-2.0");
        assert!(
            corpus["encoding"]
                .as_str()
                .expect("encoding string")
                .contains("raw compact UTF-8 JSON")
        );
        let cases = corpus["cases"].as_array().expect("corpus cases");
        assert_eq!(cases.len(), 16);

        for case in cases {
            let name = case["name"].as_str().expect("case name");
            let frame = case["frame"].as_str().expect("case frame");
            let target = case["target"].as_str().expect("case target");
            let expected_code = case["expected_code"].as_str().expect("expected code");
            let recovery_frame = case["recovery_frame"].as_str().expect("recovery frame");
            let mut session = vector_session();
            for prerequisite in case["prerequisites"]
                .as_array()
                .expect("case prerequisites")
            {
                let prerequisite = prerequisite.as_str().expect("prerequisite filename");
                let raw = fixture_bytes(prerequisite);
                let staged = session.stage(&raw).unwrap_or_else(|error| {
                    panic!("{name} prerequisite {prerequisite}: {error:?}")
                });
                let committed = session
                    .commit(staged.token)
                    .unwrap_or_else(|error| panic!("{name} prerequisite commit: {error:?}"));
                assert_eq!(
                    committed.acknowledgement.is_some(),
                    matches!(prerequisite, "baseline.json" | "delta.json")
                );
            }
            let error = session
                .stage(&fixture_bytes(frame))
                .expect_err("frozen invalid frame must not cross the Godot stage boundary");
            assert_eq!(error.code, expected_code, "invalid case {name}");
            assert!(session.pending.is_none(), "pending stage for {name}");

            let recovered = session
                .stage(&fixture_bytes(recovery_frame))
                .unwrap_or_else(|error| panic!("{name} recovery stages: {error:?}"));
            let committed = session
                .commit(recovered.token)
                .unwrap_or_else(|error| panic!("{name} recovery commits: {error:?}"));
            assert_eq!(
                committed.acknowledgement.is_some(),
                matches!(target, "baseline" | "delta"),
                "recovery ACK shape for {name}"
            );
        }
    }

    #[test]
    fn presentation_json_preserves_unsafe_integers_and_reserved_strings() {
        let presentation = presentation_safe_json(
            r#"{"safe":9007199254740991,"positive":9007199254740993,"negative":-9007199254740993,"float":1.5,"text":"__VERSE_LOSSLESS_INTEGER__:7"}"#,
        )
        .expect("valid sanitized JSON has a presentation encoding");

        assert!(presentation.contains("\"safe\":9007199254740991"));
        assert!(presentation.contains(&format!(
            "\"positive\":\"{LOSSLESS_INTEGER_PREFIX}9007199254740993\""
        )));
        assert!(presentation.contains(&format!(
            "\"negative\":\"{LOSSLESS_INTEGER_PREFIX}-9007199254740993\""
        )));
        assert!(presentation.contains("\"float\":1.5"));
        assert!(presentation.contains(&format!(
            "\"text\":\"{LOSSLESS_STRING_PREFIX}{LOSSLESS_INTEGER_PREFIX}7\""
        )));
    }
}
