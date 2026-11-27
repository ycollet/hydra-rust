use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MAX_PARAMETERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
        return Err("Parameter names require 1..64 bytes without control characters".into());
    }
    Ok(())
}

impl Parameter {
    pub(crate) fn new(name: String, default: f64, min: f64, max: f64) -> Result<Self, String> {
        if !default.is_finite()
            || !min.is_finite()
            || !max.is_finite()
            || min >= max
            || !(min..=max).contains(&default)
        {
            return Err(format!(
                "Parameter {name} requires finite bounds with min < max and a default inside them"
            ));
        }
        let parameter = Self {
            name,
            default: default as f32,
            min: min as f32,
            max: max as f32,
        };
        parameter.validate()?;
        Ok(parameter)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_name(&self.name)?;
        if !self.default.is_finite()
            || !self.min.is_finite()
            || !self.max.is_finite()
            || self.min >= self.max
            || !(self.min..=self.max).contains(&self.default)
        {
            return Err(format!(
                "Parameter {} requires finite bounds with min < max and a default inside them",
                self.name
            ));
        }
        Ok(())
    }
}

pub fn validate(parameters: &[Parameter]) -> Result<(), String> {
    if parameters.len() > MAX_PARAMETERS {
        return Err("Named parameter limit exceeded (64)".into());
    }
    for (index, parameter) in parameters.iter().enumerate() {
        parameter.validate()?;
        if parameters[..index]
            .iter()
            .any(|other| other.name == parameter.name)
        {
            return Err(format!("Duplicate parameter name: {}", parameter.name));
        }
    }
    Ok(())
}

pub(crate) fn register(
    parameters: &mut Vec<Parameter>,
    parameter: Parameter,
) -> Result<usize, String> {
    parameter.validate()?;
    if let Some(index) = parameters
        .iter()
        .position(|other| other.name == parameter.name)
    {
        if parameters[index] != parameter {
            return Err(format!(
                "Conflicting declarations for parameter {}",
                parameter.name
            ));
        }
        return Ok(index);
    }
    if parameters.len() == MAX_PARAMETERS {
        return Err("Named parameter limit exceeded (64)".into());
    }
    let index = parameters.len();
    parameters.push(parameter);
    Ok(index)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParameterValues {
    #[serde(deserialize_with = "deserialize_values")]
    values: BTreeMap<String, f32>,
}

fn deserialize_values<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, f32>, D::Error> {
    let values = BTreeMap::<String, f32>::deserialize(deserializer)?;
    if values.len() > MAX_PARAMETERS {
        return Err(serde::de::Error::custom(
            "Parameter preset limit exceeded (64)",
        ));
    }
    for (name, value) in &values {
        validate_name(name).map_err(serde::de::Error::custom)?;
        if !value.is_finite() {
            return Err(serde::de::Error::custom(
                "Parameter preset values must be finite",
            ));
        }
    }
    Ok(values)
}

impl ParameterValues {
    pub fn reconcile(parameters: &[Parameter], previous: &Self) -> Result<Self, String> {
        validate(parameters)?;
        Ok(Self {
            values: parameters
                .iter()
                .map(|parameter| {
                    let value = previous
                        .values
                        .get(&parameter.name)
                        .copied()
                        .unwrap_or(parameter.default);
                    (
                        parameter.name.clone(),
                        value.clamp(parameter.min, parameter.max),
                    )
                })
                .collect(),
        })
    }

    pub fn get(&self, name: &str) -> Option<f32> {
        self.values.get(name).copied()
    }

    pub fn set(&mut self, parameter: &Parameter, value: f32) -> Result<(), String> {
        parameter.validate()?;
        if !value.is_finite() || !(parameter.min..=parameter.max).contains(&value) {
            return Err(format!(
                "Parameter {} value lies outside its bounds",
                parameter.name
            ));
        }
        if !self.values.contains_key(&parameter.name) && self.values.len() == MAX_PARAMETERS {
            return Err("Parameter preset limit exceeded (64)".into());
        }
        self.values.insert(parameter.name.clone(), value);
        Ok(())
    }

    pub fn uniforms(&self, parameters: &[Parameter]) -> Result<[f32; MAX_PARAMETERS], String> {
        validate(parameters)?;
        let mut result = [0.0; MAX_PARAMETERS];
        for (index, parameter) in parameters.iter().enumerate() {
            let value = self.get(&parameter.name).unwrap_or(parameter.default);
            if !value.is_finite() || !(parameter.min..=parameter.max).contains(&value) {
                return Err(format!(
                    "Parameter {} value lies outside its bounds",
                    parameter.name
                ));
            }
            result[index] = value;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_follow_names_across_reordering_and_clamp_changed_bounds() {
        let previous: ParameterValues =
            serde_json::from_str(r#"{"frequency":8,"retired":3}"#).unwrap();
        let parameters = vec![
            Parameter {
                name: "angle".into(),
                default: 0.25,
                min: 0.0,
                max: 1.0,
            },
            Parameter {
                name: "frequency".into(),
                default: 2.0,
                min: 0.0,
                max: 5.0,
            },
        ];
        let values = ParameterValues::reconcile(&parameters, &previous).unwrap();
        assert_eq!(values.get("frequency"), Some(5.0));
        assert_eq!(values.get("angle"), Some(0.25));
        assert_eq!(values.get("retired"), None);
        assert_eq!(&values.uniforms(&parameters).unwrap()[..2], &[0.25, 5.0]);
        assert_eq!(previous.get("frequency"), Some(8.0));
        let mut changed = values.clone();
        assert!(changed.set(&parameters[0], f32::NAN).is_err());
        assert!(changed.set(&parameters[0], 2.0).is_err());
        assert_eq!(changed, values);
        changed.set(&parameters[0], 0.5).unwrap();
        let restored: ParameterValues =
            serde_json::from_str(&serde_json::to_string(&changed).unwrap()).unwrap();
        assert_eq!(restored, changed);
    }

    #[test]
    fn declarations_and_imported_presets_reject_invalid_or_excessive_data() {
        let parameter = Parameter {
            name: "frequency".into(),
            default: 2.0,
            min: 0.0,
            max: 5.0,
        };
        let mut declarations = Vec::new();
        assert_eq!(register(&mut declarations, parameter.clone()).unwrap(), 0);
        assert_eq!(register(&mut declarations, parameter.clone()).unwrap(), 0);
        assert!(
            register(
                &mut declarations,
                Parameter {
                    default: 3.0,
                    ..parameter.clone()
                }
            )
            .is_err()
        );
        assert_eq!(declarations, vec![parameter]);
        for json in [r#"{"":1}"#, r#"{"bad\nname":1}"#, r#"{"frequency":1e100}"#] {
            assert!(
                serde_json::from_str::<ParameterValues>(json).is_err(),
                "{json}"
            );
        }
        let oversized: BTreeMap<_, _> = (0..65).map(|index| (format!("p{index}"), 0.5)).collect();
        assert!(
            serde_json::from_str::<ParameterValues>(&serde_json::to_string(&oversized).unwrap())
                .is_err()
        );
    }
}
