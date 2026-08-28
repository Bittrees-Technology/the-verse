// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;
use serde::ser::{
    self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};

use crate::error::{ErrorCode, Result, VerifyError};

pub(crate) const DOMAIN: &[u8] = b"the-verse/interest-view/v1\0";

pub(crate) fn fixed_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let node = value.serialize(NodeSerializer).map_err(map_error)?;
    let mut output = Vec::new();
    write_node(&node, &mut output);
    Ok(output)
}

pub(crate) fn digest<T: Serialize>(value: &T) -> Result<String> {
    let canonical = fixed_json(value)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN);
    hasher.update(&canonical);
    Ok(hasher.finalize().to_hex().to_string())
}

fn map_error(error: CanonicalError) -> VerifyError {
    VerifyError::new(error.code, error.message)
}

#[derive(Debug)]
struct CanonicalError {
    code: ErrorCode,
    message: String,
}

impl CanonicalError {
    fn float(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidFloat,
            message: message.into(),
        }
    }
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CanonicalError {}

impl ser::Error for CanonicalError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self {
            code: ErrorCode::Serialization,
            message: message.to_string(),
        }
    }
}

type CanonicalResult<T> = std::result::Result<T, CanonicalError>;

#[derive(Debug)]
enum Node {
    Null,
    Bool(bool),
    Signed(i128),
    Unsigned(u128),
    String(String),
    Sequence(Vec<Node>),
    Object(BTreeMap<String, Node>),
}

struct NodeSerializer;

impl ser::Serializer for NodeSerializer {
    type Ok = Node;
    type Error = CanonicalError;
    type SerializeSeq = SequenceBuilder;
    type SerializeTuple = SequenceBuilder;
    type SerializeTupleStruct = SequenceBuilder;
    type SerializeTupleVariant = TupleVariantBuilder;
    type SerializeMap = MapBuilder;
    type SerializeStruct = MapBuilder;
    type SerializeStructVariant = StructVariantBuilder;

    fn serialize_bool(self, value: bool) -> CanonicalResult<Node> {
        Ok(Node::Bool(value))
    }

    fn serialize_i8(self, value: i8) -> CanonicalResult<Node> {
        Ok(Node::Signed(i128::from(value)))
    }

    fn serialize_i16(self, value: i16) -> CanonicalResult<Node> {
        Ok(Node::Signed(i128::from(value)))
    }

    fn serialize_i32(self, value: i32) -> CanonicalResult<Node> {
        Ok(Node::Signed(i128::from(value)))
    }

    fn serialize_i64(self, value: i64) -> CanonicalResult<Node> {
        Ok(Node::Signed(i128::from(value)))
    }

    fn serialize_i128(self, value: i128) -> CanonicalResult<Node> {
        Ok(Node::Signed(value))
    }

    fn serialize_u8(self, value: u8) -> CanonicalResult<Node> {
        Ok(Node::Unsigned(u128::from(value)))
    }

    fn serialize_u16(self, value: u16) -> CanonicalResult<Node> {
        Ok(Node::Unsigned(u128::from(value)))
    }

    fn serialize_u32(self, value: u32) -> CanonicalResult<Node> {
        Ok(Node::Unsigned(u128::from(value)))
    }

    fn serialize_u64(self, value: u64) -> CanonicalResult<Node> {
        Ok(Node::Unsigned(u128::from(value)))
    }

    fn serialize_u128(self, value: u128) -> CanonicalResult<Node> {
        Ok(Node::Unsigned(value))
    }

    fn serialize_f32(self, value: f32) -> CanonicalResult<Node> {
        fixed_float(f64::from(value))
    }

    fn serialize_f64(self, value: f64) -> CanonicalResult<Node> {
        fixed_float(value)
    }

    fn serialize_char(self, value: char) -> CanonicalResult<Node> {
        Ok(Node::String(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> CanonicalResult<Node> {
        Ok(Node::String(value.to_owned()))
    }

    fn serialize_bytes(self, value: &[u8]) -> CanonicalResult<Node> {
        Ok(Node::Sequence(
            value
                .iter()
                .map(|byte| Node::Unsigned(u128::from(*byte)))
                .collect(),
        ))
    }

    fn serialize_none(self) -> CanonicalResult<Node> {
        Ok(Node::Null)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> CanonicalResult<Node> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> CanonicalResult<Node> {
        Ok(Node::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> CanonicalResult<Node> {
        Ok(Node::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> CanonicalResult<Node> {
        Ok(Node::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> CanonicalResult<Node> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> CanonicalResult<Node> {
        let mut object = BTreeMap::new();
        object.insert(variant.to_owned(), value.serialize(NodeSerializer)?);
        Ok(Node::Object(object))
    }

    fn serialize_seq(self, length: Option<usize>) -> CanonicalResult<SequenceBuilder> {
        Ok(SequenceBuilder::new(length))
    }

    fn serialize_tuple(self, length: usize) -> CanonicalResult<SequenceBuilder> {
        Ok(SequenceBuilder::new(Some(length)))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        length: usize,
    ) -> CanonicalResult<SequenceBuilder> {
        Ok(SequenceBuilder::new(Some(length)))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        length: usize,
    ) -> CanonicalResult<TupleVariantBuilder> {
        Ok(TupleVariantBuilder {
            variant: variant.to_owned(),
            values: Vec::with_capacity(length),
        })
    }

    fn serialize_map(self, length: Option<usize>) -> CanonicalResult<MapBuilder> {
        Ok(MapBuilder::new(length))
    }

    fn serialize_struct(self, _name: &'static str, length: usize) -> CanonicalResult<MapBuilder> {
        Ok(MapBuilder::new(Some(length)))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        length: usize,
    ) -> CanonicalResult<StructVariantBuilder> {
        Ok(StructVariantBuilder {
            variant: variant.to_owned(),
            values: BTreeMap::new(),
            expected: length,
        })
    }
}

fn fixed_float(value: f64) -> CanonicalResult<Node> {
    const I64_EXCLUSIVE_MAX: f64 = 9_223_372_036_854_775_808.0;
    if !value.is_finite() {
        return Err(CanonicalError::float("non-finite floating-point value"));
    }
    let scaled = (value * 1_000_000.0).round();
    if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled >= I64_EXCLUSIVE_MAX {
        return Err(CanonicalError::float(
            "fixed_1e6 floating-point value is outside i64 range",
        ));
    }
    let scaled = if scaled == 0.0 { 0 } else { scaled as i64 };
    Ok(Node::Sequence(vec![
        Node::String("fixed_1e6".to_owned()),
        Node::Signed(i128::from(scaled)),
    ]))
}

struct SequenceBuilder {
    values: Vec<Node>,
}

impl SequenceBuilder {
    fn new(length: Option<usize>) -> Self {
        Self {
            values: Vec::with_capacity(length.unwrap_or(0)),
        }
    }
}

impl SerializeSeq for SequenceBuilder {
    type Ok = Node;
    type Error = CanonicalError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> CanonicalResult<()> {
        self.values.push(value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> CanonicalResult<Node> {
        Ok(Node::Sequence(self.values))
    }
}

impl SerializeTuple for SequenceBuilder {
    type Ok = Node;
    type Error = CanonicalError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> CanonicalResult<()> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> CanonicalResult<Node> {
        SerializeSeq::end(self)
    }
}

impl SerializeTupleStruct for SequenceBuilder {
    type Ok = Node;
    type Error = CanonicalError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> CanonicalResult<()> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> CanonicalResult<Node> {
        SerializeSeq::end(self)
    }
}

struct TupleVariantBuilder {
    variant: String,
    values: Vec<Node>,
}

impl SerializeTupleVariant for TupleVariantBuilder {
    type Ok = Node;
    type Error = CanonicalError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> CanonicalResult<()> {
        self.values.push(value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> CanonicalResult<Node> {
        let mut object = BTreeMap::new();
        object.insert(self.variant, Node::Sequence(self.values));
        Ok(Node::Object(object))
    }
}

struct MapBuilder {
    values: BTreeMap<String, Node>,
    pending_key: Option<String>,
}

impl MapBuilder {
    fn new(length: Option<usize>) -> Self {
        let _ = length;
        Self {
            values: BTreeMap::new(),
            pending_key: None,
        }
    }
}

impl SerializeMap for MapBuilder {
    type Ok = Node;
    type Error = CanonicalError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> CanonicalResult<()> {
        let Node::String(key) = key.serialize(NodeSerializer)? else {
            return Err(ser::Error::custom("canonical JSON map key is not a string"));
        };
        self.pending_key = Some(key);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> CanonicalResult<()> {
        let key = self
            .pending_key
            .take()
            .ok_or_else(|| ser::Error::custom("map value has no key"))?;
        self.values.insert(key, value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> CanonicalResult<Node> {
        if self.pending_key.is_some() {
            return Err(ser::Error::custom("map key has no value"));
        }
        Ok(Node::Object(self.values))
    }
}

impl SerializeStruct for MapBuilder {
    type Ok = Node;
    type Error = CanonicalError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> CanonicalResult<()> {
        self.values
            .insert(key.to_owned(), value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> CanonicalResult<Node> {
        Ok(Node::Object(self.values))
    }
}

struct StructVariantBuilder {
    variant: String,
    values: BTreeMap<String, Node>,
    expected: usize,
}

impl SerializeStructVariant for StructVariantBuilder {
    type Ok = Node;
    type Error = CanonicalError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> CanonicalResult<()> {
        self.values
            .insert(key.to_owned(), value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> CanonicalResult<Node> {
        let _ = self.expected;
        let mut object = BTreeMap::new();
        object.insert(self.variant, Node::Object(self.values));
        Ok(Node::Object(object))
    }
}

fn write_node(node: &Node, output: &mut Vec<u8>) {
    match node {
        Node::Null => output.extend_from_slice(b"null"),
        Node::Bool(true) => output.extend_from_slice(b"true"),
        Node::Bool(false) => output.extend_from_slice(b"false"),
        Node::Signed(value) => output.extend_from_slice(value.to_string().as_bytes()),
        Node::Unsigned(value) => output.extend_from_slice(value.to_string().as_bytes()),
        Node::String(value) => write_string(value, output),
        Node::Sequence(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_node(value, output);
            }
            output.push(b']');
        }
        Node::Object(values) => {
            output.push(b'{');
            for (index, (key, value)) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_string(key, output);
                output.push(b':');
                write_node(value, output);
            }
            output.push(b'}');
        }
    }
}

fn write_string(value: &str, output: &mut Vec<u8>) {
    output.push(b'"');
    for character in value.chars() {
        match character {
            '"' => output.extend_from_slice(b"\\\""),
            '\\' => output.extend_from_slice(b"\\\\"),
            '\u{0008}' => output.extend_from_slice(b"\\b"),
            '\t' => output.extend_from_slice(b"\\t"),
            '\n' => output.extend_from_slice(b"\\n"),
            '\u{000c}' => output.extend_from_slice(b"\\f"),
            '\r' => output.extend_from_slice(b"\\r"),
            '\u{0000}'..='\u{001f}' => {
                let value = u32::from(character);
                output.extend_from_slice(format!("\\u00{value:02x}").as_bytes());
            }
            _ => {
                let mut buffer = [0; 4];
                output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    output.push(b'"');
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::fixed_json;
    use crate::ErrorCode;

    #[derive(Serialize)]
    struct Fixture<'a> {
        z: f64,
        a: &'a str,
        f: f32,
    }

    #[test]
    fn canonical_json_sorts_and_fixes_floats() {
        let bytes = fixed_json(&Fixture {
            z: -0.000_000_5,
            a: "x\n\u{0001}é",
            f: 1.234_567_5,
        })
        .expect("canonical serialization succeeds");
        assert_eq!(
            String::from_utf8(bytes).expect("canonical JSON is UTF-8"),
            r#"{"a":"x\n\u0001é","f":["fixed_1e6",1234568],"z":["fixed_1e6",-1]}"#
        );
    }

    #[test]
    fn canonical_json_rejects_bad_floats() {
        let error = fixed_json(&f64::INFINITY).expect_err("infinity is rejected");
        assert_eq!(error.code(), ErrorCode::InvalidFloat);
        let error = fixed_json(&1.0e20_f64).expect_err("scaled overflow is rejected");
        assert_eq!(error.code(), ErrorCode::InvalidFloat);
    }
}
