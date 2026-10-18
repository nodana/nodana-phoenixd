use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
pub fn canonical(value: &impl Serialize) -> Vec<u8> {
    // Value maps are sorted; identity and HPKE contexts contain ASCII fields.
    serde_json::to_vec(&serde_json::to_value(value).expect("serializable value")).expect("JSON")
}
pub fn parse_json(bytes: &[u8]) -> Result<Value> {
    struct Unique(Value);
    impl<'de> serde::Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Unique;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("JSON without duplicate fields")
                }
                fn visit_map<M: serde::de::MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<Unique, M::Error> {
                    let mut out = serde_json::Map::new();
                    while let Some((key, value)) = map.next_entry::<String, Unique>()? {
                        if out.insert(key, value.0).is_some() {
                            return Err(serde::de::Error::custom("duplicate field"));
                        }
                    }
                    Ok(Unique(Value::Object(out)))
                }
                fn visit_seq<S: serde::de::SeqAccess<'de>>(
                    self,
                    mut sequence: S,
                ) -> std::result::Result<Unique, S::Error> {
                    let mut values = Vec::new();
                    while let Some(value) = sequence.next_element::<Unique>()? {
                        values.push(value.0);
                    }
                    Ok(Unique(Value::Array(values)))
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    v: bool,
                ) -> std::result::Result<Unique, E> {
                    Ok(Unique(Value::Bool(v)))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                    Ok(Unique(Value::String(v.into())))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                    Ok(Unique(json_number(v.into())))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                    Ok(Unique(json_number(v.into())))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                    Ok(Unique(
                        serde_json::Number::from_f64(v)
                            .map(Value::Number)
                            .ok_or_else(|| E::custom("invalid number"))?,
                    ))
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
            }
            d.deserialize_any(Visitor)
        }
    }
    fn json_number(v: serde_json::Number) -> Value {
        Value::Number(v)
    }
    Ok(serde_json::from_slice::<Unique>(bytes)?.0)
}

pub fn digest(bytes: impl AsRef<[u8]>) -> Vec<u8> {
    Sha256::digest(bytes.as_ref()).to_vec()
}
