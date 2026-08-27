// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;

use serde::de::{DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserializer, Serialize};
use serde_json::{Map, Number, Value};

use crate::ResourceLimits;
use crate::error::{ErrorCode, Result, VerifyError};

const DUPLICATE_PREFIX: &str = "__verse_duplicate_key__:";

pub(crate) fn parse_exact<T>(raw: &[u8], limits: &ResourceLimits) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    if raw.len() > limits.max_frame_bytes {
        return Err(VerifyError::new(
            ErrorCode::FrameTooLarge,
            "wire frame exceeds max_frame_bytes",
        ));
    }

    let mut deserializer = serde_json::Deserializer::from_slice(raw);
    let value = StrictValueSeed
        .deserialize(&mut deserializer)
        .map_err(|error| map_json_error(&error))?;
    deserializer.end().map_err(|error| map_json_error(&error))?;
    check_resources(&value, limits)?;

    let typed: T = serde_json::from_value(value.clone()).map_err(|error| {
        let message = error.to_string();
        let code = if message.contains("unknown field") {
            ErrorCode::UnknownField
        } else {
            ErrorCode::InvalidJson
        };
        VerifyError::new(code, format!("wire shape: {message}"))
    })?;
    let known = serde_json::to_value(&typed).map_err(|error| {
        VerifyError::new(
            ErrorCode::Serialization,
            format!("typed wire serialization: {error}"),
        )
    })?;
    ensure_no_unknown_fields(&value, &known, "$")?;
    Ok(typed)
}

fn map_json_error(error: &serde_json::Error) -> VerifyError {
    let message = error.to_string();
    if let Some(offset) = message.find(DUPLICATE_PREFIX) {
        let key = message[offset + DUPLICATE_PREFIX.len()..]
            .split(" at line")
            .next()
            .unwrap_or("<unknown>");
        VerifyError::new(
            ErrorCode::DuplicateKey,
            format!("duplicate object key {key:?}"),
        )
    } else {
        VerifyError::new(ErrorCode::InvalidJson, message)
    }
}

fn ensure_no_unknown_fields(input: &Value, known: &Value, path: &str) -> Result<()> {
    match (input, known) {
        (Value::Object(input), Value::Object(known)) => {
            for (key, value) in input {
                let Some(known_value) = known.get(key) else {
                    return Err(VerifyError::new(
                        ErrorCode::UnknownField,
                        format!("unknown field {path}.{key}"),
                    ));
                };
                ensure_no_unknown_fields(value, known_value, &format!("{path}.{key}"))?;
            }
        }
        (Value::Array(input), Value::Array(known)) => {
            if input.len() != known.len() {
                return Err(VerifyError::new(
                    ErrorCode::InvalidJson,
                    format!("typed array length changed at {path}"),
                ));
            }
            for (index, (value, known_value)) in input.iter().zip(known).enumerate() {
                ensure_no_unknown_fields(value, known_value, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_resources(value: &Value, limits: &ResourceLimits) -> Result<()> {
    struct Usage {
        values: usize,
        string_bytes: usize,
    }

    fn visit(
        value: &Value,
        depth: usize,
        limits: &ResourceLimits,
        usage: &mut Usage,
    ) -> Result<()> {
        if depth > limits.max_json_depth {
            return Err(VerifyError::new(
                ErrorCode::ResourceLimit,
                "JSON nesting exceeds max_json_depth",
            ));
        }
        usage.values = usage.values.checked_add(1).ok_or_else(|| {
            VerifyError::new(ErrorCode::ResourceLimit, "JSON value count overflow")
        })?;
        if usage.values > limits.max_json_values {
            return Err(VerifyError::new(
                ErrorCode::ResourceLimit,
                "JSON value count exceeds max_json_values",
            ));
        }
        match value {
            Value::String(value) => add_string(value, limits, usage)?,
            Value::Array(values) => {
                if values.len() > limits.max_collection_len {
                    return Err(VerifyError::new(
                        ErrorCode::ResourceLimit,
                        "JSON array exceeds max_collection_len",
                    ));
                }
                for value in values {
                    visit(value, depth + 1, limits, usage)?;
                }
            }
            Value::Object(values) => {
                if values.len() > limits.max_collection_len {
                    return Err(VerifyError::new(
                        ErrorCode::ResourceLimit,
                        "JSON object exceeds max_collection_len",
                    ));
                }
                for (key, value) in values {
                    add_string(key, limits, usage)?;
                    visit(value, depth + 1, limits, usage)?;
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
        Ok(())
    }

    fn add_string(value: &str, limits: &ResourceLimits, usage: &mut Usage) -> Result<()> {
        if value.len() > limits.max_string_bytes {
            return Err(VerifyError::new(
                ErrorCode::ResourceLimit,
                "one JSON string exceeds max_string_bytes",
            ));
        }
        usage.string_bytes = usage.string_bytes.checked_add(value.len()).ok_or_else(|| {
            VerifyError::new(ErrorCode::ResourceLimit, "JSON string byte count overflow")
        })?;
        if usage.string_bytes > limits.max_total_string_bytes {
            return Err(VerifyError::new(
                ErrorCode::ResourceLimit,
                "JSON strings exceed max_total_string_bytes",
            ));
        }
        Ok(())
    }

    visit(
        value,
        1,
        limits,
        &mut Usage {
            values: 0,
            string_bytes: 0,
        },
    )
}

struct StrictValueSeed;

impl<'de> DeserializeSeed<'de> for StrictValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictValueSeed.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(4096));
        while let Some(value) = sequence.next_element_seed(StrictValueSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        let mut keys = HashSet::with_capacity(map.size_hint().unwrap_or(0).min(4096));
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!("{DUPLICATE_PREFIX}{key}")));
            }
            let value = map.next_value_seed(StrictValueSeed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
