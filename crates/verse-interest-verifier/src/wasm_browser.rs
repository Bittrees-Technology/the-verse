// SPDX-License-Identifier: Apache-2.0

//! String-only browser ABI around the portable verifier.

use serde::{Deserialize, Serialize};
use verse_protocol::SessionRole;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use crate::{InterestVerifier, StageKind, StageToken, VerifierConfig, VerifyError};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserConfig {
    expected_role: String,
    #[serde(default)]
    expected_player_id: Option<String>,
    world_schema_version: String,
    event_schema_version: String,
    content_schema_version: String,
    content_manifest_version: String,
    expected_content_hash: String,
    expected_universe_id: String,
    expected_celestial_registry_hash: String,
    expected_universe_manifest_hash: String,
}

#[derive(Debug, Serialize)]
struct AdapterResponse<'a> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_json: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    acknowledgement_json: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
}

impl AdapterResponse<'_> {
    fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"ok":false,"code":"serialization","detail":"adapter response serialization failed"}"#
                .to_owned()
        })
    }
}

/// Browser-owned state machine. Every exported value crossing the WASM ABI is
/// a UTF-8 JSON string, preserving protocol integers and number spelling until
/// the portable verifier has accepted the original frame.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[derive(Debug)]
pub struct BrowserInterestVerifier {
    verifier: Option<InterestVerifier>,
    init_error: Option<String>,
    pending: Option<(String, StageToken)>,
    next_stage_id: u64,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl BrowserInterestVerifier {
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(constructor))]
    pub fn new(config_json: &str) -> Self {
        match parse_config(config_json)
            .and_then(|config| InterestVerifier::new(config).map_err(|error| error.to_string()))
        {
            Ok(verifier) => Self {
                verifier: Some(verifier),
                init_error: None,
                pending: None,
                next_stage_id: 1,
            },
            Err(error) => Self {
                verifier: None,
                init_error: Some(error),
                pending: None,
                next_stage_id: 1,
            },
        }
    }

    /// Report whether construction produced a usable verifier.
    pub fn readiness(&self) -> String {
        match &self.init_error {
            None => AdapterResponse {
                ok: true,
                stage_id: None,
                kind: None,
                message_json: None,
                acknowledgement_json: None,
                code: None,
                detail: None,
            }
            .encode(),
            Some(detail) => AdapterResponse {
                ok: false,
                stage_id: None,
                kind: None,
                message_json: None,
                acknowledgement_json: None,
                code: Some("initialization"),
                detail: Some(detail),
            }
            .encode(),
        }
    }

    /// Stage one raw server frame. A second frame cannot be staged until the
    /// returned identifier is committed or discarded.
    pub fn stage(&mut self, raw_json: &str) -> String {
        let Some(verifier) = self.verifier.as_mut() else {
            return self.init_failure();
        };
        if self.pending.is_some() {
            return failure("pending_stage", "a browser transition is already pending");
        }
        let token = match verifier.stage(raw_json.as_bytes()) {
            Ok(token) => token,
            Err(error) => return verifier_failure(&error),
        };
        let stage_id = self.next_stage_id.to_string();
        self.next_stage_id = self.next_stage_id.wrapping_add(1).max(1);
        let kind = verifier
            .pending_kind()
            .map(stage_kind)
            .expect("successful stage creates pending kind");
        let message_json = verifier
            .pending_sanitized_json()
            .expect("successful stage creates sanitized JSON")
            .to_owned();
        self.pending = Some((stage_id.clone(), token));
        AdapterResponse {
            ok: true,
            stage_id: Some(stage_id),
            kind: Some(kind),
            message_json: Some(&message_json),
            acknowledgement_json: None,
            code: None,
            detail: None,
        }
        .encode()
    }

    /// Commit the exact pending stage and return the verifier-owned ACK JSON.
    pub fn commit(&mut self, stage_id: String) -> String {
        let Some(verifier) = self.verifier.as_mut() else {
            return self.init_failure();
        };
        let Some((expected_id, token)) = self.pending.take() else {
            return failure("invalid_stage_token", "no browser transition is pending");
        };
        if expected_id != stage_id {
            self.pending = Some((expected_id, token));
            return failure(
                "invalid_stage_token",
                "browser stage identifier does not match the pending transition",
            );
        }
        match verifier.commit(token) {
            Ok(outcome) => AdapterResponse {
                ok: true,
                stage_id: Some(stage_id),
                kind: Some(stage_kind(outcome.kind)),
                message_json: None,
                acknowledgement_json: outcome.acknowledgement_json.as_deref(),
                code: None,
                detail: None,
            }
            .encode(),
            Err(error) => verifier_failure(&error),
        }
    }

    /// Discard the pending stage without installing state or producing an ACK.
    pub fn discard(&mut self, stage_id: String) -> String {
        let Some(verifier) = self.verifier.as_mut() else {
            return self.init_failure();
        };
        let Some((expected_id, token)) = self.pending.take() else {
            return failure("invalid_stage_token", "no browser transition is pending");
        };
        if expected_id != stage_id {
            self.pending = Some((expected_id, token));
            return failure(
                "invalid_stage_token",
                "browser stage identifier does not match the pending transition",
            );
        }
        match verifier.discard(token) {
            Ok(()) => AdapterResponse {
                ok: true,
                stage_id: Some(stage_id),
                kind: None,
                message_json: None,
                acknowledgement_json: None,
                code: None,
                detail: None,
            }
            .encode(),
            Err(error) => verifier_failure(&error),
        }
    }

    /// Reset the connection binding and invalidate any outstanding stage.
    pub fn reset(&mut self) -> String {
        let Some(verifier) = self.verifier.as_mut() else {
            return self.init_failure();
        };
        verifier.reset();
        self.pending = None;
        AdapterResponse {
            ok: true,
            stage_id: None,
            kind: None,
            message_json: None,
            acknowledgement_json: None,
            code: None,
            detail: None,
        }
        .encode()
    }
}

impl BrowserInterestVerifier {
    fn init_failure(&self) -> String {
        failure(
            "initialization",
            self.init_error
                .as_deref()
                .unwrap_or("browser verifier is unavailable"),
        )
    }
}

fn parse_config(raw: &str) -> Result<VerifierConfig, String> {
    let parsed: BrowserConfig =
        serde_json::from_str(raw).map_err(|error| format!("invalid browser config: {error}"))?;
    let role = match (parsed.expected_role.as_str(), parsed.expected_player_id) {
        ("spectator", None) => SessionRole::Spectator,
        ("player", Some(player_id)) if !player_id.is_empty() => SessionRole::Player { player_id },
        ("spectator", Some(_)) => {
            return Err("spectator browser verifier config forbids expected_player_id".to_owned());
        }
        ("player", None | Some(_)) => {
            return Err("player browser verifier config requires expected_player_id".to_owned());
        }
        _ => return Err("expected_role must be spectator or player".to_owned()),
    };
    let parse_version = |name: &str, value: &str| {
        value
            .parse::<u32>()
            .map_err(|_| format!("{name} must be an unsigned 32-bit decimal string"))
    };
    Ok(VerifierConfig::new(
        role,
        parse_version("world_schema_version", &parsed.world_schema_version)?,
        parse_version("event_schema_version", &parsed.event_schema_version)?,
        parse_version("content_schema_version", &parsed.content_schema_version)?,
        parsed.content_manifest_version,
        parsed.expected_content_hash,
        parsed.expected_universe_id,
        parsed.expected_celestial_registry_hash,
        parsed.expected_universe_manifest_hash,
    ))
}

const fn stage_kind(kind: StageKind) -> &'static str {
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

fn verifier_failure(error: &VerifyError) -> String {
    failure(error.code().as_str(), error.detail())
}

fn failure(code: &str, detail: &str) -> String {
    AdapterResponse {
        ok: false,
        stage_id: None,
        kind: None,
        message_json: None,
        acknowledgement_json: None,
        code: Some(code),
        detail: Some(detail),
    }
    .encode()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::BrowserInterestVerifier;

    fn verifier() -> BrowserInterestVerifier {
        let config = json!({
            "expected_role": "spectator",
            "world_schema_version": "20",
            "event_schema_version": "16",
            "content_schema_version": "11",
            "content_manifest_version": "p1.5.0",
            "expected_content_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "expected_universe_id": "universe-test",
            "expected_celestial_registry_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "expected_universe_manifest_hash": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        })
        .to_string();
        BrowserInterestVerifier::new(&config)
    }

    fn response(raw: &str) -> Value {
        serde_json::from_str(raw).expect("adapter response is JSON")
    }

    #[test]
    fn initialization_is_fail_closed_and_string_configured() {
        let invalid = BrowserInterestVerifier::new(r#"{"expected_role":"player"}"#);
        let readiness = response(&invalid.readiness());
        assert_eq!(readiness["ok"], false);
        assert_eq!(readiness["code"], "initialization");

        assert_eq!(response(&verifier().readiness())["ok"], true);

        let player = BrowserInterestVerifier::new(
            &json!({
                "expected_role": "player",
                "expected_player_id": "player-local",
                "world_schema_version": "20",
                "event_schema_version": "16",
                "content_schema_version": "11",
                "content_manifest_version": "p1.5.0",
                "expected_content_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "expected_universe_id": "universe-test",
                "expected_celestial_registry_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "expected_universe_manifest_hash": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            })
            .to_string(),
        );
        assert_eq!(response(&player.readiness())["ok"], true);

        let ambiguous = BrowserInterestVerifier::new(
            &json!({
                "expected_role": "spectator",
                "expected_player_id": "player-local",
                "world_schema_version": "20",
                "event_schema_version": "16",
                "content_schema_version": "11",
                "content_manifest_version": "p1.5.0",
                "expected_content_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "expected_universe_id": "universe-test",
                "expected_celestial_registry_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "expected_universe_manifest_hash": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            })
            .to_string(),
        );
        assert_eq!(response(&ambiguous.readiness())["ok"], false);
    }

    #[test]
    fn welcome_is_staged_once_and_committed_by_opaque_string_id() {
        let mut verifier = verifier();
        let welcome = json!({
            "type": "welcome",
            "protocol_version": 18,
            "projection_schema_version": 4,
            "world_schema_version": 20,
            "event_schema_version": 16,
            "content_schema_version": 11,
            "content_manifest_version": "p1.5.0",
            "celestial_registry_schema_version": 1,
            "universe_manifest_schema_version": 4,
            "interest_schema_version": 2,
            "session_role": {"kind": "spectator"},
            "server_name": "test"
        });
        let welcome = welcome.to_string();
        let staged = response(&verifier.stage(&welcome));
        assert_eq!(staged["ok"], true);
        assert_eq!(staged["kind"], "welcome");
        assert_eq!(staged["stage_id"], "1");
        assert!(staged["message_json"].as_str().is_some());

        let blocked = response(&verifier.stage(&welcome));
        assert_eq!(blocked["code"], "pending_stage");
        let wrong = response(&verifier.commit("2".to_owned()));
        assert_eq!(wrong["code"], "invalid_stage_token");
        let committed = response(&verifier.commit("1".to_owned()));
        assert_eq!(committed["ok"], true);
        assert!(committed.get("acknowledgement_json").is_none());
    }

    #[test]
    fn malformed_frames_never_create_a_committable_stage() {
        let mut verifier = verifier();
        let failed = response(&verifier.stage("{not-json"));
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["code"], "invalid_json");
        assert_eq!(
            response(&verifier.commit("1".to_owned()))["code"],
            "invalid_stage_token"
        );
        assert_eq!(response(&verifier.reset())["ok"], true);
    }

    #[test]
    fn discard_requires_the_exact_stage_identifier() {
        let mut verifier = verifier();
        let welcome = json!({
            "type": "welcome",
            "protocol_version": 18,
            "projection_schema_version": 4,
            "world_schema_version": 20,
            "event_schema_version": 16,
            "content_schema_version": 11,
            "content_manifest_version": "p1.5.0",
            "celestial_registry_schema_version": 1,
            "universe_manifest_schema_version": 4,
            "interest_schema_version": 2,
            "session_role": {"kind": "spectator"},
            "server_name": "test"
        });
        assert_eq!(response(&verifier.stage(&welcome.to_string()))["ok"], true);
        assert_eq!(
            response(&verifier.discard("wrong".to_owned()))["code"],
            "invalid_stage_token"
        );
        assert_eq!(response(&verifier.discard("1".to_owned()))["ok"], true);
    }
}
