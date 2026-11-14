//! Imported enrollment policy, spatial admission and regional point coverage.

use std::collections::BTreeMap;

use gnorix_core::biometrics::fingerprint::{
    self, CombinationConfig, GridBaselineConfig, SpatialPolicy,
};
use serde_json::{Map, Value};

use super::{Result, enrollment_profile::ProfileParameters, input::CombinationInput};

/// Validate the imported selected subsets and retain their exact reconstruction policy.
pub(super) fn parameters(
    input: &CombinationInput,
    grid: &GridBaselineConfig,
) -> Result<ProfileParameters> {
    let controls = combination_controls(&input.metadata)?;
    validate_selected_features(input, &controls)?;
    ProfileParameters::new(&controls, grid.padding_steps)
}

fn combination_controls(metadata: &Map<String, Value>) -> Result<CombinationConfig> {
    let config = metadata
        .get("config")
        .and_then(Value::as_object)
        .ok_or("combination report requires config")?;
    let expected = [
        "min_points",
        "max_points",
        "min_priority",
        "radius_units",
        "spatial",
        "max_work",
    ];
    if config.len() != expected.len()
        || config.keys().any(|name| !expected.contains(&name.as_str()))
    {
        return Err("unsupported combination configuration fields".into());
    }
    let controls = CombinationConfig {
        min_points: integer(config, "min_points")?,
        max_points: integer(config, "max_points")?,
        min_priority: numeric(config, "min_priority")?,
        radius_units: numeric(config, "radius_units")?,
        spatial: spatial_policy(config.get("spatial"))?,
        max_work: integer(config, "max_work")?,
    };
    controls.validate()?;
    Ok(controls)
}

fn numeric(fields: &Map<String, Value>, name: &str) -> Result<f64> {
    fields
        .get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{name} must be numeric").into())
}

fn spatial_policy(value: Option<&Value>) -> Result<SpatialPolicy> {
    let fields = spatial_fields(value, &["min_anchor_points", "min_anchor_distance_units"])?;
    let policy = SpatialPolicy {
        min_anchor_points: integer(fields, "min_anchor_points")?,
        min_anchor_distance_units: numeric(fields, "min_anchor_distance_units")?,
    };
    policy.validate()?;
    Ok(policy)
}

fn spatial_fields<'a>(
    value: Option<&'a Value>,
    expected: &[&str],
) -> Result<&'a Map<String, Value>> {
    let fields = value
        .and_then(Value::as_object)
        .ok_or("report requires a spatial parameter object")?;
    if fields.len() != expected.len()
        || fields.keys().any(|name| !expected.contains(&name.as_str()))
    {
        return Err("unsupported spatial parameter fields".into());
    }
    Ok(fields)
}

fn validate_selected_features(
    input: &CombinationInput,
    controls: &CombinationConfig,
) -> Result<()> {
    let mut remaining_work = controls.max_work;
    reserve_validation_work(&mut remaining_work, input.points.len())?;
    let mut points = BTreeMap::new();
    for point in &input.points {
        if points.insert(point.id, point).is_some() {
            return Err(format!("duplicate feature ID {}", point.id).into());
        }
    }
    let mut coverage = declared_regions(input, &points, controls, &mut remaining_work)?;
    for (index, combination) in input.combinations.iter().enumerate() {
        let count = combination.feature_ids.len();
        if count < controls.min_points || count > controls.max_points {
            return Err(format!(
                "combination {index} has a point count outside the declared policy"
            )
            .into());
        }
        reserve_validation_work(&mut remaining_work, count)?;
        let mut positions = Vec::with_capacity(count);
        for id in &combination.feature_ids {
            let point = points
                .get(id)
                .ok_or_else(|| format!("combination {index} selects unknown feature {id}"))?;
            if point.priority < controls.min_priority {
                return Err(format!(
                    "combination {index} selects feature {id} below the configured minimum priority"
                )
                .into());
            }
            positions.push(point.position);
        }
        if remaining_work == 0 {
            return Err(fingerprint::CombinationError::WorkLimit.into());
        }
        let (satisfies, used) = fingerprint::satisfies_spatial_requirements(
            &positions,
            input.unit_px,
            &controls.spatial,
            remaining_work,
        )?;
        reserve_validation_work(&mut remaining_work, used)?;
        if !satisfies {
            return Err(format!(
                "combination {index} does not satisfy the declared source-point geometry"
            )
            .into());
        }
        if combination.regions.is_empty() {
            return Err(format!("combination {index} has no selected region").into());
        }
        let membership_work = count
            .checked_add(1)
            .and_then(|amount| amount.checked_mul(combination.regions.len()))
            .ok_or(fingerprint::CombinationError::WorkLimit)?;
        reserve_validation_work(&mut remaining_work, membership_work)?;
        for region in &combination.regions {
            let members = coverage
                .get_mut(region)
                .ok_or_else(|| format!("combination {index} refers to unknown region {region}"))?;
            for id in &combination.feature_ids {
                let covered = members.get_mut(id).ok_or_else(|| {
                    format!("combination {index} selects feature {id} outside region {region}")
                })?;
                *covered = true;
            }
        }
    }
    for (region, members) in coverage {
        reserve_validation_work(
            &mut remaining_work,
            members
                .len()
                .checked_add(1)
                .ok_or(fingerprint::CombinationError::WorkLimit)?,
        )?;
        for (feature_id, covered) in members {
            if !covered {
                return Err(
                    fingerprint::CombinationError::UncoverablePoint { region, feature_id }.into(),
                );
            }
        }
    }
    Ok(())
}

type RegionCoverage = BTreeMap<usize, BTreeMap<usize, bool>>;

fn declared_regions(
    input: &CombinationInput,
    points: &BTreeMap<usize, &fingerprint::CombinationPoint>,
    controls: &CombinationConfig,
    remaining_work: &mut usize,
) -> Result<RegionCoverage> {
    let regions = input
        .metadata
        .get("regions")
        .and_then(Value::as_array)
        .filter(|regions| !regions.is_empty())
        .ok_or("enrollment report requires selected regions")?;
    let mut coverage = BTreeMap::new();
    for region in regions {
        reserve_validation_work(remaining_work, 1)?;
        let fields = region
            .as_object()
            .ok_or("selected region must be an object")?;
        let id = integer(fields, "id")?;
        let ids = fields
            .get("feature_ids")
            .and_then(Value::as_array)
            .ok_or("selected region requires feature IDs")?;
        if ids.len() < controls.min_points.max(controls.spatial.min_anchor_points) {
            return Err(format!("region {id} has too few points for the declared policy").into());
        }
        reserve_validation_work(remaining_work, ids.len())?;
        let mut members = BTreeMap::new();
        for item in ids {
            let feature_id = item
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("regional feature ID must be an unsigned integer")?;
            let point = points
                .get(&feature_id)
                .ok_or_else(|| format!("region {id} selects unknown feature {feature_id}"))?;
            if point.priority < controls.min_priority {
                return Err(format!(
                    "region {id} selects feature {feature_id} below the minimum priority"
                )
                .into());
            }
            if members.insert(feature_id, false).is_some() {
                return Err(format!("region {id} repeats feature {feature_id}").into());
            }
        }
        if coverage.insert(id, members).is_some() {
            return Err(format!("duplicate selected region ID {id}").into());
        }
    }
    Ok(coverage)
}

fn reserve_validation_work(remaining: &mut usize, amount: usize) -> Result<()> {
    *remaining = remaining
        .checked_sub(amount)
        .ok_or(fingerprint::CombinationError::WorkLimit)?;
    Ok(())
}

fn integer(fields: &Map<String, Value>, name: &str) -> Result<usize> {
    usize::try_from(
        fields
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{name} must be an unsigned integer"))?,
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnorix_core::biometrics::geometry::Point;
    use serde_json::json;

    fn imported() -> CombinationInput {
        crate::input::parse(json!({
            "schema":"gnorix.feature-combinations.v6", "unit_px":4.0,
            "bounds":{"min":[0,0],"max":[12,12]},
            "config":{"min_points":3,"max_points":6,"min_priority":0.5,
                "radius_units":30.0,
                "spatial":{"min_anchor_points":3,
                    "min_anchor_distance_units":0.5},"max_work":10000},
            "points":[
                {"id":0,"kind":"ending","position":[0,0],"priority":0.5},
                {"id":1,"kind":"ending","position":[10,0],"priority":0.9},
                {"id":2,"kind":"ending","position":[0,10],"priority":0.8},
                {"id":3,"kind":"ending","position":[2,2],"priority":0.49}],
            "regions":[{"id":0,"feature_ids":[0,1,2]}],
            "combinations":[{"feature_ids":[0,1,2],"score":0.5,"regions":[0]}]
        }))
        .unwrap()
    }

    fn grid() -> GridBaselineConfig {
        GridBaselineConfig {
            padding_steps: 1,
            max_baselines: 10,
            max_work: 10000,
        }
    }

    #[test]
    fn retained_policy_is_canonical_bound_and_independent_of_capture_scale_or_work() {
        let mut input = imported();
        let first = parameters(&input, &grid()).unwrap();
        let loaded = ProfileParameters::decode(first.encoded(), first.profile_id()).unwrap();
        assert_eq!(
            loaded.spatial(),
            SpatialPolicy {
                min_anchor_points: 3,
                min_anchor_distance_units: 0.5,
            }
        );
        assert_eq!(loaded.uncertainty_steps(), 1);
        assert_eq!(loaded.controls(50).unwrap().min_priority, 0.5);
        input.metadata["config"]["max_work"] = json!(20000);
        assert_eq!(
            first.profile_id(),
            parameters(&input, &grid()).unwrap().profile_id()
        );
        input.metadata["bounds"] = json!({"min":[0,0],"max":[6,6]});
        assert_eq!(
            first.profile_id(),
            parameters(&input, &grid()).unwrap().profile_id()
        );
        assert_eq!(loaded.spatial().min_anchor_distance_units, 0.5);
        input.metadata["config"]["min_priority"] = json!(0.0);
        let zero = parameters(&input, &grid()).unwrap();
        assert_ne!(first.profile_id(), zero.profile_id());
        input.metadata["config"]["min_priority"] = json!(-0.0);
        assert_eq!(
            zero.profile_id(),
            parameters(&input, &grid()).unwrap().profile_id()
        );
        input.metadata["config"]["spatial"]["min_anchor_distance_units"] = json!(1.5);
        assert_ne!(
            zero.profile_id(),
            parameters(&input, &grid()).unwrap().profile_id()
        );
    }

    #[test]
    fn imported_subsets_must_meet_priority_and_the_shared_geometry_predicate() {
        let mut input = imported();
        assert!(parameters(&input, &grid()).is_ok());
        input.combinations[0].feature_ids = vec![0, 1, 2, 3];
        assert!(
            parameters(&input, &grid())
                .unwrap_err()
                .to_string()
                .contains("priority")
        );
        input.points[3].priority = 0.8;
        input.metadata["regions"][0]["feature_ids"] = json!([0, 1, 2, 3]);
        assert!(parameters(&input, &grid()).is_ok()); // Interior fourth point is covered.
        input.points[3].position = Point::new(10.0, 1.0).unwrap();
        // The outside point is too close to point1 to make both anchors;
        // choosing either one as an anchor leaves the other outside the hull.
        assert!(
            parameters(&input, &grid())
                .unwrap_err()
                .to_string()
                .contains("geometry")
        );
        let mut input = imported();
        input.points[1].position = Point::new(1.0, 0.0).unwrap();
        input.points[2].position = Point::new(0.0, 1.0).unwrap();
        assert!(
            parameters(&input, &grid())
                .unwrap_err()
                .to_string()
                .contains("geometry")
        );
    }

    #[test]
    fn imports_verify_direct_distance_and_reject_old_or_unknown_fields() {
        let mut input = imported();
        input.metadata["config"]["spatial"]["min_anchor_distance_units"] = json!(-1.0);
        assert!(parameters(&input, &grid()).is_err());
        let mut input = imported();
        input.metadata["config"]["spatial"]["min_anchor_points"] = json!(0);
        assert!(parameters(&input, &grid()).is_err());
        let mut input = imported();
        input.metadata["config"]["spatial"]["extra"] = json!(true);
        assert!(parameters(&input, &grid()).is_err());
        let mut input = imported();
        input.metadata["config"]
            .as_object_mut()
            .unwrap()
            .remove("spatial");
        assert!(parameters(&input, &grid()).is_err());
    }

    #[test]
    fn region_can_exceed_subset_maximum_but_each_combination_must_fit() {
        let mut input = imported();
        input.metadata["config"]["max_points"] = json!(3);
        input.points[3].priority = 0.8;
        input.points[3].position = Point::new(10.0, 10.0).unwrap();
        input.metadata["regions"][0]["feature_ids"] = json!([0, 1, 2, 3]);
        input.combinations.push(input.combinations[0].clone());
        input.combinations[1].feature_ids = vec![1, 2, 3];
        assert!(parameters(&input, &grid()).is_ok());

        input.combinations[1].feature_ids = vec![0, 1, 2, 3];
        assert!(
            parameters(&input, &grid())
                .unwrap_err()
                .to_string()
                .contains("point count")
        );
    }

    #[test]
    fn imported_geometry_validation_has_one_actual_aggregate_work_budget() {
        let mut input = imported();
        input.points[3].priority = 0.8;
        input.points[3].position = Point::new(10.0, 10.0).unwrap();
        input.metadata["regions"][0]["feature_ids"] = json!([0, 1, 2, 3]);
        input.combinations.push(input.combinations[0].clone());
        input.combinations[1].feature_ids = vec![1, 2, 3];
        let spatial = combination_controls(&input.metadata).unwrap().spatial;
        let mut required = input.points.len() + 2 * (1 + 4);
        for subset in &input.combinations {
            let positions: Vec<_> = subset
                .feature_ids
                .iter()
                .map(|&id| input.points[id].position)
                .collect();
            let (valid, used) = fingerprint::satisfies_spatial_requirements(
                &positions,
                input.unit_px,
                &spatial,
                10000,
            )
            .unwrap();
            assert!(valid);
            required += positions.len() + used + 1 + positions.len();
        }
        input.metadata["config"]["max_work"] = json!(required - 1);
        let error = parameters(&input, &grid()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<fingerprint::CombinationError>(),
            Some(&fingerprint::CombinationError::WorkLimit)
        );
        input.metadata["config"]["max_work"] = json!(required);
        assert!(parameters(&input, &grid()).is_ok());
    }

    #[test]
    fn imports_cannot_remove_feature_coverage_or_region_membership() {
        let mut input = imported();
        input.points[3].priority = 0.8;
        input.metadata["regions"][0]["feature_ids"] = json!([0, 1, 2, 3]);
        let error = parameters(&input, &grid()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<fingerprint::CombinationError>(),
            Some(&fingerprint::CombinationError::UncoverablePoint {
                region: 0,
                feature_id: 3
            })
        );
        for regions in [Vec::new(), vec![999]] {
            let mut input = imported();
            input.combinations[0].regions = regions;
            assert!(parameters(&input, &grid()).is_err());
        }
        let mut input = imported();
        input.metadata.remove("regions");
        assert!(parameters(&input, &grid()).is_err());
        let mut input = imported();
        input.metadata["regions"][0]["feature_ids"] = json!([0, 1, 3]);
        input.points[3].priority = 0.8;
        assert!(
            parameters(&input, &grid())
                .unwrap_err()
                .to_string()
                .contains("outside region")
        );
    }
}
