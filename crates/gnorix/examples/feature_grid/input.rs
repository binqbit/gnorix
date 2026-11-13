//! Bounded JSON decoding and typed feature-combination input validation.

use std::io::Read;

use gnorix::biometrics::{
    fingerprint::{CombinationPoint, FeatureCombination, FeatureKind},
    geometry::Point,
};
use serde_json::{Map, Value};

use super::Result;

#[derive(Debug)]
pub(super) struct CombinationInput {
    pub(super) metadata: Map<String, Value>,
    pub(super) points: Vec<CombinationPoint>,
    pub(super) combinations: Vec<FeatureCombination>,
    pub(super) unit_px: f64,
}

pub(super) fn read(reader: &mut impl Read, limit: usize) -> Result<CombinationInput> {
    let read_limit = u64::try_from(limit)?
        .checked_add(1)
        .ok_or("input limit overflow")?;
    let mut encoded = Vec::new();
    reader.take(read_limit).read_to_end(&mut encoded)?;
    if encoded.len() > limit {
        return Err(format!("input JSON exceeds {limit} bytes").into());
    }
    parse(serde_json::from_slice(&encoded)?)
}

pub(super) fn parse(document: Value) -> Result<CombinationInput> {
    let Value::Object(mut metadata) = document else {
        return Err("input JSON must be an object".into());
    };
    if metadata.get("schema").and_then(Value::as_str) != Some("gnorix.feature-combinations.v2") {
        return Err("expected schema gnorix.feature-combinations.v2".into());
    }
    let unit_px = finite(metadata.get("unit_px"), "unit_px")?;
    if unit_px <= 0.0 {
        return Err("unit_px must be positive".into());
    }
    let points = metadata.remove("points").ok_or("missing points")?;
    let combinations = metadata
        .remove("combinations")
        .ok_or("missing combinations")?;
    let points = array(&points, "points")?
        .iter()
        .enumerate()
        .map(|(index, point)| parse_point(point, &format!("points[{index}]")))
        .collect::<Result<_>>()?;
    let combinations = array(&combinations, "combinations")?
        .iter()
        .enumerate()
        .map(|(index, item)| parse_combination(item, &format!("combinations[{index}]")))
        .collect::<Result<_>>()?;
    Ok(CombinationInput {
        metadata,
        points,
        combinations,
        unit_px,
    })
}

fn parse_point(value: &Value, path: &str) -> Result<CombinationPoint> {
    let id = usize::try_from(integer(value.get("id"), &format!("{path}.id"))?)?;
    let kind = match value.get("kind").and_then(Value::as_str) {
        Some("ending") => FeatureKind::Ending,
        Some("bifurcation") => FeatureKind::Bifurcation,
        Some("junction") => FeatureKind::Junction,
        Some("isolated") => FeatureKind::Isolated,
        Some("cycle") => FeatureKind::Cycle,
        _ => return Err(format!("{path}.kind is not a known feature kind").into()),
    };
    let coordinates = array(
        value
            .get("position")
            .ok_or_else(|| format!("missing {path}.position"))?,
        path,
    )?;
    if coordinates.len() != 2 {
        return Err(format!("{path}.position must have exactly two coordinates").into());
    }
    let position = Point::new(
        finite(coordinates.first(), &format!("{path}.position[0]"))?,
        finite(coordinates.get(1), &format!("{path}.position[1]"))?,
    )?;
    let priority = unit_interval(value.get("priority"), &format!("{path}.priority"))?;
    Ok(CombinationPoint {
        id,
        kind,
        position,
        priority,
    })
}

fn parse_combination(value: &Value, path: &str) -> Result<FeatureCombination> {
    let ids = value
        .get("feature_ids")
        .ok_or_else(|| format!("missing {path}.feature_ids"))?;
    let feature_ids = array(ids, path)?
        .iter()
        .map(|id| {
            Ok(usize::try_from(integer(
                Some(id),
                &format!("{path}.feature_ids"),
            )?)?)
        })
        .collect::<Result<_>>()?;
    let score = unit_interval(value.get("score"), &format!("{path}.score"))?;
    let regions = value
        .get("regions")
        .ok_or_else(|| format!("missing {path}.regions"))?;
    let regions = array(regions, path)?
        .iter()
        .map(|region| {
            Ok(usize::try_from(integer(
                Some(region),
                &format!("{path}.regions"),
            )?)?)
        })
        .collect::<Result<_>>()?;
    Ok(FeatureCombination {
        feature_ids,
        score,
        regions,
    })
}

fn array<'a>(value: &'a Value, path: &str) -> Result<&'a [Value]> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{path} must be an array").into())
}

fn integer(value: Option<&Value>, path: &str) -> Result<u64> {
    value
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{path} must be a nonnegative integer").into())
}

fn finite(value: Option<&Value>, path: &str) -> Result<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| format!("{path} must be a finite number").into())
}

fn unit_interval(value: Option<&Value>, path: &str) -> Result<f64> {
    let number = finite(value, path)?;
    if !(0.0..=1.0).contains(&number) {
        return Err(format!("{path} must be between zero and one").into());
    }
    Ok(number)
}
