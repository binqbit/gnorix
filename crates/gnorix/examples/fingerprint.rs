//! Extract one PNG capture to inspectable local stages and graph geometry.

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use gnorix::biometrics::{
    Capture,
    fingerprint::{self, CurveGeometry, ExtractionConfig, Fingerprint, NodeKind, RidgeMap},
};
use image::{GrayImage, ImageReader};
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn Error>> {
    let (input, output) = arguments()?;
    if output.exists() {
        return Err("output directory already exists".into());
    }
    let config = ExtractionConfig::default();
    let source = read_image(&input, config.max_pixels)?;
    let capture = Capture::new(
        source.width(),
        source.height(),
        source.as_raw().clone(),
        None,
        None,
    )?;

    let started = Instant::now();
    let result = fingerprint::extract(&capture, &config)?;
    let elapsed = started.elapsed();
    fs::create_dir_all(&output)?;
    save_bundle(&output, &source, &result, &config)?;
    println!(
        "Full-source unit: {:.3}px; {:.3}px measured / {:.3}px traversed ridge length; {} / {} links measured",
        result.scale.unit_px,
        result.scale.coverage.measured_length_px,
        result.scale.coverage.total_length_px,
        result.scale.coverage.measured_links,
        result.scale.coverage.total_links,
    );
    println!(
        "Provisional bootstrap: period {:.3}px from {} profiles; final width observations come from the full pass",
        result.bootstrap.ridge_period_px,
        result.bootstrap.profiles.len(),
    );
    println!(
        "{} nodes, {} edges, {} isolated cycles; {:.3}s extraction\n{}",
        result.graph.nodes.len(),
        result.graph.edges.len(),
        result.graph.cycles.len(),
        elapsed.as_secs_f64(),
        output.display()
    );
    Ok(())
}

fn arguments() -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err("usage: fingerprint <input.png> <new-output-directory>".into());
    }
    Ok((PathBuf::from(&arguments[0]), PathBuf::from(&arguments[1])))
}

fn read_image(path: &Path, max_pixels: usize) -> Result<GrayImage, Box<dyn Error>> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    opaque_grayscale(reader.decode()?, max_pixels)
}

fn opaque_grayscale(
    image: image::DynamicImage,
    max_pixels: usize,
) -> Result<GrayImage, Box<dyn Error>> {
    if u64::from(image.width()) * u64::from(image.height()) > max_pixels as u64 {
        return Err("image exceeds the source-pixel budget".into());
    }
    if image.color().has_alpha() && image.to_rgba16().pixels().any(|pixel| pixel[3] != u16::MAX) {
        return Err(
            "scanner input must be opaque; transparent pixels require an explicit validity mask"
                .into(),
        );
    }
    Ok(image.into_luma8())
}

fn save_bundle(
    root: &Path,
    source: &GrayImage,
    result: &Fingerprint,
    config: &ExtractionConfig,
) -> Result<(), Box<dyn Error>> {
    source.save(root.join("source.png"))?;
    save_palette(root, result)?;
    save_values(
        root,
        "normalized.png",
        result,
        result
            .normalized
            .iter()
            .map(|&v| ((v + 3.0) / 6.0 * 255.0).round() as u8),
    )?;
    save_values(
        root,
        "denoised.png",
        result,
        result
            .denoised
            .iter()
            .map(|&v| ((v + 3.0) / 6.0 * 255.0).round() as u8),
    )?;
    save_values(
        root,
        "coherence.png",
        result,
        result.coherence.iter().map(|&v| (v * 255.0).round() as u8),
    )?;
    save_values(
        root,
        "usable.png",
        result,
        result
            .binary
            .usable()
            .iter()
            .map(|&v| if v { 255 } else { 0 }),
    )?;
    save_map(root, "binary.png", &result.binary)?;
    save_map(root, "skeleton.png", &result.skeleton)?;
    save_map(root, "measurement-skeleton.png", &result.scale.skeleton)?;
    save_values(
        root,
        "pattern-support.png",
        result,
        result
            .scale
            .pattern_support
            .iter()
            .map(|&v| if v { 255 } else { 0 }),
    )?;
    let snapshot = snapshot(result, config)?;
    fs::write(
        root.join("features.json"),
        serde_json::to_vec_pretty(&snapshot)?,
    )?;
    Ok(())
}

fn save_palette(root: &Path, result: &Fingerprint) -> Result<(), Box<dyn Error>> {
    for (filename, values) in [
        ("ridge-level.png", &result.palette.ridge_gray),
        ("valley-level.png", &result.palette.valley_gray),
    ] {
        save_values(
            root,
            filename,
            result,
            values.iter().map(|v| v.round() as u8),
        )?;
    }
    save_values(
        root,
        "normalization-support.png",
        result,
        result
            .palette
            .usable
            .iter()
            .map(|&v| if v { 255 } else { 0 }),
    )
}

fn save_values(
    root: &Path,
    filename: &str,
    result: &Fingerprint,
    values: impl Iterator<Item = u8>,
) -> Result<(), Box<dyn Error>> {
    let image = GrayImage::from_raw(
        result.binary.width() as u32,
        result.binary.height() as u32,
        values.collect(),
    )
    .ok_or("invalid stage dimensions")?;
    image.save(root.join(filename))?;
    Ok(())
}

fn save_map(root: &Path, filename: &str, map: &RidgeMap) -> Result<(), Box<dyn Error>> {
    let values = map
        .ridges()
        .iter()
        .map(|&v| if v { 0 } else { 255 })
        .collect();
    GrayImage::from_raw(map.width() as u32, map.height() as u32, values)
        .ok_or("invalid ridge dimensions")?
        .save(root.join(filename))?;
    Ok(())
}

fn snapshot(
    result: &Fingerprint,
    config: &ExtractionConfig,
) -> Result<Value, fingerprint::FingerprintError> {
    let nodes: Vec<_> = result.graph.nodes.iter().enumerate().map(|(id, node)| json!({
        "id":id, "x":node.position.x(), "y":node.position.y(),
        "kind": match node.kind { NodeKind::Ending=>"ending",NodeKind::Junction=>"junction",NodeKind::Isolated=>"isolated" },
        "touches_unobserved":node.touches_unobserved,
    })).collect();
    let edges: Vec<_> = result
        .graph
        .edges
        .iter()
        .zip(&result.edge_geometry)
        .enumerate()
        .map(|(id, (edge, geometry))| {
            let (geometry, error) = geometry_json(geometry, result.scale.unit_px);
            json!({"id":id,"start":edge.start,"end":edge.end,
            "points":edge.points.iter().map(|p| [p.x(),p.y()]).collect::<Vec<_>>(),
            "geometry":geometry,"geometry_error":error})
        })
        .collect();
    let cycles: Vec<_> = result
        .graph
        .cycles
        .iter()
        .zip(&result.cycle_geometry)
        .enumerate()
        .map(|(id, (cycle, geometry))| {
            let (geometry, error) = geometry_json(geometry, result.scale.unit_px);
            json!({"id":id,"points":cycle.points.iter().map(|p| [p.x(),p.y()]).collect::<Vec<_>>(),
            "geometry":geometry,"geometry_error":error})
        })
        .collect();
    Ok(json!({
        "schema":"gnorix.ridge-extraction.v1", "width":result.binary.width(),"height":result.binary.height(),
        "algorithm":"Native local dark/light reference normalization, spatial denoising, orientation-directed ridge contrast, Zhang-Suen thinning, topology tracing and Gaussian-smoothed curve measurements",
        "config":{"bootstrap_grid_locations":config.bootstrap_grid_locations,"min_contrast":config.min_contrast,"min_coherence":config.min_coherence},
        "bootstrap":bootstrap_snapshot(result)?,
        "scale":full_scale_snapshot(result),
        "normalization":{"radius_px":result.palette.radius_px,"method":"local-two-class-midpoint","reference_encoding":"rounded-u8"},
        "coordinate_system":"source image pixels, x right and y down; no rescaling",
        "qualifications":["Extraction controls are not biometric acceptance tolerances.","Topology is measured from the processed image, not confirmed anatomy.","No matching, key reconstruction or real-capture qualification is provided."],
        "images":{"source":"source.png","normalized":"normalized.png","denoised":"denoised.png","coherence":"coherence.png","usable":"usable.png","binary":"binary.png","skeleton":"skeleton.png","ridge_level":"ridge-level.png","valley_level":"valley-level.png","normalization_support":"normalization-support.png","measurement_skeleton":"measurement-skeleton.png","pattern_support":"pattern-support.png"},
        "nodes":nodes,"edges":edges,"cycles":cycles,
    }))
}

fn bootstrap_snapshot(result: &Fingerprint) -> Result<Value, fingerprint::FingerprintError> {
    let scale = &result.bootstrap;
    let consistency = scale.width_consistency()?;
    Ok(json!({
        "ridge_width_px":scale.ridge_width_px,
        "valley_width_px":scale.valley_width_px,
        "ridge_period_px":scale.ridge_period_px,
        "ridge_width_mad_px":scale.ridge_width_mad_px,
        "ridge_period_mad_px":scale.ridge_period_mad_px,
        "mean_ridge_width_px":scale.mean_ridge_width_px,
        "mean_valley_width_px":scale.mean_valley_width_px,
        "mean_ridge_gray":scale.mean_ridge_gray,
        "mean_valley_gray":scale.mean_valley_gray,
        "examined_locations":scale.examined_locations,
        "profiles":scale.profiles.iter().map(|profile| json!({
            "ridges":profile.ridges.iter().map(|ridge| json!({
                "start":[ridge.start.x(),ridge.start.y()],
                "end":[ridge.end.x(),ridge.end.y()],
            })).collect::<Vec<_>>(),
            "ridge_mean_gray":profile.ridge_mean_gray,
            "valley_mean_gray":profile.valley_mean_gray,
        })).collect::<Vec<_>>(),
        "width_consistency":consistency.iter().map(|quality|json!({
            "relative_tolerance":quality.relative_tolerance,
            "accepted_measurements":quality.accepted_measurements,
            "total_measurements":quality.total_measurements,
            "fraction":quality.fraction,
        })).collect::<Vec<_>>(),
    }))
}

fn full_scale_snapshot(result: &Fingerprint) -> Value {
    let scale = &result.scale;
    let coverage = &scale.coverage;
    json!({
        "method":"raw-centerline-link-width",
        "unit_px":scale.unit_px,
        "coverage":{
            "total_length_px":coverage.total_length_px,
            "measured_length_px":coverage.measured_length_px,
            "total_links":coverage.total_links,
            "measured_links":coverage.measured_links,
            "isolated_pixels":coverage.isolated_pixels,
        },
        "observations":scale.observations.iter().map(|observation|json!({
            "link":observation.link,
            "start":[observation.span.start.x(),observation.span.start.y()],
            "end":[observation.span.end.x(),observation.span.end.y()],
            "represented_length_px":observation.represented_length_px,
            "mean_gray":observation.mean_gray,
        })).collect::<Vec<_>>(),
        "width_consistency":result.width_consistency.iter().map(|quality|json!({
            "relative_tolerance":quality.relative_tolerance,
            "accepted_measurements":quality.accepted_measurements,
            "total_measurements":quality.total_measurements,
            "fraction":quality.fraction,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, Rgba};

    #[test]
    fn transparent_hidden_pixels_are_not_interpreted_as_scanner_evidence() {
        let transparent =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([0, 0, 0, 0])));
        let almost_opaque =
            DynamicImage::ImageRgba16(ImageBuffer::from_pixel(2, 2, Rgba([0, 0, 0, 65_534])));
        assert!(opaque_grayscale(transparent, 4).is_err());
        assert!(opaque_grayscale(almost_opaque, 4).is_err());
    }

    #[test]
    fn opaque_images_respect_the_pixel_budget() {
        let image =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([100, 100, 100, 255])));
        assert!(opaque_grayscale(image.clone(), 3).is_err());
        assert_eq!(opaque_grayscale(image, 4).unwrap().as_raw(), &[100; 4]);
    }
}

fn geometry_json(
    result: &Result<CurveGeometry, fingerprint::FingerprintError>,
    unit_px: f64,
) -> (Value, Option<String>) {
    match result {
        Ok(value) => (
            json!({"arc_length":value.arc_length,"chord_length":value.chord_length,
            "arc_length_units":value.arc_length/unit_px,"chord_length_units":value.chord_length/unit_px,
            "midpoint":[value.midpoint.x(),value.midpoint.y()],
            "turning":value.turning.iter().map(|p|json!({"arc_fraction":p.arc_fraction,"angle_radians":p.angle_radians})).collect::<Vec<_>>() }),
            None,
        ),
        Err(error) => (Value::Null, Some(error.to_string())),
    }
}
