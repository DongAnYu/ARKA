use std::num::NonZeroU32;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

pub const DEFAULT_MAX_LEARNING_ITEMS: u32 = 20;

/// Selection purpose for Default generation. Foundations includes essential
/// definitions and prerequisites. Custom focus prompts are intentionally absent.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationPurpose {
    #[default]
    Balanced,
    Foundations,
    Explanations,
    PracticalApplication,
}

/// Immutable configuration captured when a Default job starts.
///
/// The positive integer type rejects zero, negative, fractional, and non-numeric
/// inputs during IPC deserialization, before provider initialization. This is a
/// whole-note learning-target ceiling, enforced before Stage B generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GenerationOptions {
    max_learning_items: NonZeroU32,
    purpose: GenerationPurpose,
}

// Serde's defaulted struct visitor also accepts sequences such as []. The IPC
// contract requires an object, so only map input is allowed here.
impl<'de> Deserialize<'de> for GenerationOptions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OptionsVisitor;

        impl<'de> Visitor<'de> for OptionsVisitor {
            type Value = GenerationOptions;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a generation options object")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut options = GenerationOptions::default();
                let mut maximum_seen = false;
                let mut purpose_seen = false;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "max_learning_items" => {
                            if maximum_seen {
                                return Err(de::Error::duplicate_field("max_learning_items"));
                            }
                            options.max_learning_items = map.next_value()?;
                            maximum_seen = true;
                        }
                        "purpose" => {
                            if purpose_seen {
                                return Err(de::Error::duplicate_field("purpose"));
                            }
                            options.purpose = map.next_value()?;
                            purpose_seen = true;
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["max_learning_items", "purpose"],
                            ))
                        }
                    }
                }
                Ok(options)
            }
        }

        deserializer.deserialize_map(OptionsVisitor)
    }
}

impl Default for GenerationOptions {
    fn default() -> Self {
        Self {
            max_learning_items: NonZeroU32::new(DEFAULT_MAX_LEARNING_ITEMS)
                .expect("the default maximum must be positive"),
            purpose: GenerationPurpose::default(),
        }
    }
}

impl GenerationOptions {
    pub fn resolve(options: Option<Self>) -> Self {
        options.unwrap_or_default()
    }

    pub fn max_learning_items(self) -> u32 {
        self.max_learning_items.get()
    }

    pub fn purpose(self) -> GenerationPurpose {
        self.purpose
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn omitted_options_and_individual_fields_resolve_consistently() {
        let defaults = GenerationOptions::resolve(None);
        assert_eq!(defaults.max_learning_items(), 20);
        assert_eq!(defaults.purpose(), GenerationPurpose::Balanced);
        assert_eq!(
            serde_json::from_value::<GenerationOptions>(json!({})).unwrap(),
            defaults
        );

        let maximum_only: GenerationOptions =
            serde_json::from_value(json!({ "max_learning_items": 7 })).unwrap();
        assert_eq!(maximum_only.max_learning_items(), 7);
        assert_eq!(maximum_only.purpose(), GenerationPurpose::Balanced);

        let purpose_only: GenerationOptions =
            serde_json::from_value(json!({ "purpose": "foundations" })).unwrap();
        assert_eq!(purpose_only.max_learning_items(), 20);
        assert_eq!(purpose_only.purpose(), GenerationPurpose::Foundations);
    }

    #[test]
    fn all_supported_purposes_round_trip_with_an_explicit_maximum() {
        for (wire, purpose) in [
            ("balanced", GenerationPurpose::Balanced),
            ("foundations", GenerationPurpose::Foundations),
            ("explanations", GenerationPurpose::Explanations),
            (
                "practical_application",
                GenerationPurpose::PracticalApplication,
            ),
        ] {
            let input = json!({ "max_learning_items": 100, "purpose": wire });
            let options: GenerationOptions = serde_json::from_value(input.clone()).unwrap();
            assert_eq!(options.max_learning_items(), 100);
            assert_eq!(options.purpose(), purpose);
            assert_eq!(GenerationOptions::resolve(Some(options)), options);
            assert_eq!(serde_json::to_value(options).unwrap(), input);
        }
    }

    #[test]
    fn invalid_maximums_and_purposes_are_rejected_at_deserialization() {
        for maximum in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!("20"),
            Value::Null,
            json!(true),
            json!([]),
            json!(4294967296_u64),
        ] {
            assert!(
                serde_json::from_value::<GenerationOptions>(
                    json!({ "max_learning_items": maximum })
                )
                .is_err(),
                "accepted invalid maximum {maximum}"
            );
        }
        for purpose in [
            json!("definitions"),
            json!("auto"),
            json!("Balanced"),
            Value::Null,
            json!(20),
        ] {
            assert!(
                serde_json::from_value::<GenerationOptions>(json!({ "purpose": purpose })).is_err(),
                "accepted invalid purpose {purpose}"
            );
        }
        for input in [
            json!([]),
            json!("invalid"),
            json!({ "maxLearningItems": 5 }),
        ] {
            assert!(
                serde_json::from_value::<GenerationOptions>(input.clone()).is_err(),
                "accepted invalid options {input}"
            );
        }
        assert!(serde_json::from_str::<GenerationOptions>(
            r#"{"max_learning_items":20,"max_learning_items":10}"#
        )
        .is_err());
    }
}
