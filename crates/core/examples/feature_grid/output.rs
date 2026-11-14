//! Manifest serialization and bounded streaming of complete positional variants.

use std::{
    fs::{self, OpenOptions},
    io::{self, BufWriter, Write},
    path::Path,
};

use gnorix::biometrics::{
    fingerprint::{FeatureKind, GridShape, GridShapeConfig, GridShapes, GridVariant},
    geometry::Point,
};
use serde_json::{Value, json};

use super::{Result, input::CombinationInput};

#[derive(Debug)]
pub(super) struct WrittenReport {
    pub(super) variants: u64,
    pub(super) colliding_variants: u64,
    pub(super) bytes: usize,
}

pub(super) fn require_new_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("output path already exists".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn manifest(
    input_path: &Path,
    input: &CombinationInput,
    config: &GridShapeConfig,
    prepared: &GridShapes,
) -> Value {
    let points: Vec<_> = input
        .points
        .iter()
        .map(|point| {
            json!({
                "id": point.id, "kind": kind_name(point.kind),
                "position": xy(point.position), "priority": point.priority,
            })
        })
        .collect();
    let combinations: Vec<_> = input
        .combinations
        .iter()
        .map(|item| {
            json!({
                "feature_ids": item.feature_ids, "score": item.score, "regions": item.regions,
            })
        })
        .collect();
    let shapes: Vec<_> = prepared.shapes().iter().map(shape_snapshot).collect();
    json!({
        "schema": "gnorix.feature-grid.v1",
        "source": input.metadata.get("source"),
        "input_path": input_path.to_string_lossy(), "input_metadata": input.metadata,
        "unit_px": prepared.spacing_px(),
        "config": {
            "uncertainty_steps": config.uncertainty_steps,
            "max_variants": config.max_variants, "max_work": config.max_work,
        },
        "policy": {
            "orientation": "longest convex-boundary edge horizontal; body below",
            "origin": "top-right corner of oriented source bounding box",
            "coordinates": "nonnegative columns left, rows down",
            "row_stagger": "leftward distance/R = column + ((row + phase) % 2)/2",
            "row_spacing": "sqrt(3)*R/2",
            "padding": "fixed uncertainty_steps for the complete shape traversal",
            "variants": "complete hex-step product; collisions and repeated geometry retained",
            "recentering": "none per variant",
        },
        "points": points, "combinations": combinations, "shapes": shapes,
        "planned_variant_count": prepared.variant_count(),
        "preparation_work": prepared.preparation_work(),
    })
}

fn kind_name(kind: FeatureKind) -> &'static str {
    match kind {
        FeatureKind::Ending => "ending",
        FeatureKind::Bifurcation => "bifurcation",
        FeatureKind::Junction => "junction",
        FeatureKind::Isolated => "isolated",
        FeatureKind::Cycle => "cycle",
    }
}

fn xy(point: Point) -> [f64; 2] {
    [point.x(), point.y()]
}

fn shape_snapshot(shape: &GridShape) -> Value {
    json!({
        "combination_index": shape.combination_index, "feature_ids": shape.feature_ids,
        "frame": {
            "boundary": shape.frame.boundary, "top_edge": shape.frame.top_edge,
            "origin": xy(shape.frame.origin), "x_axis": xy(shape.frame.x_axis),
        },
        "aligned": shape.aligned.iter().copied().map(xy).collect::<Vec<_>>(),
        "snapped": shape.snapped.iter().map(|point| [point.q, point.r]).collect::<Vec<_>>(),
        "baseline": shape.baseline.iter().map(|point| [point.column, point.row]).collect::<Vec<_>>(),
        "phase": shape.phase,
    })
}

fn variant_snapshot(variant: &GridVariant) -> Value {
    json!({
        "shape_index": variant.shape_index,
        "positions": variant.positions.iter().map(|point| [point.column, point.row]).collect::<Vec<_>>(),
        "collisions": variant.collisions.iter().map(|collision| json!({
            "position": [collision.position.column, collision.position.row],
            "feature_ids": collision.feature_ids,
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn write_new(
    path: &Path,
    manifest: &Value,
    prepared: &GridShapes,
    limit: usize,
) -> Result<WrittenReport> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let result = {
        let mut writer = LimitedWriter {
            inner: BufWriter::new(file),
            written: 0,
            limit,
        };
        stream_report(&mut writer, manifest, prepared).and_then(|(variants, colliding_variants)| {
            writer.flush()?;
            Ok(WrittenReport {
                variants,
                colliding_variants,
                bytes: writer.written,
            })
        })
    };
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_file(path) {
            return Err(format!("{error}; could not remove incomplete output: {cleanup}").into());
        }
        return Err(error);
    }
    result
}

fn stream_report(
    writer: &mut impl Write,
    manifest: &Value,
    prepared: &GridShapes,
) -> Result<(u64, u64)> {
    let fields = manifest
        .as_object()
        .ok_or("report manifest must be an object")?;
    writer.write_all(b"{")?;
    for (name, value) in fields {
        serde_json::to_writer(&mut *writer, name)?;
        writer.write_all(b":")?;
        serde_json::to_writer(&mut *writer, value)?;
        writer.write_all(b",")?;
    }
    writer.write_all(b"\"variants\":[")?;
    let mut count = 0_u64;
    let mut colliding = 0_u64;
    for variant in prepared.variants() {
        let variant = variant?;
        if count > 0 {
            writer.write_all(b",")?;
        }
        serde_json::to_writer(&mut *writer, &variant_snapshot(&variant))?;
        count = count.checked_add(1).ok_or("variant count overflow")?;
        colliding += u64::from(!variant.collisions.is_empty());
    }
    if count != prepared.variant_count() {
        return Err("variant traversal did not produce the complete planned count".into());
    }
    writeln!(
        writer,
        "],\"variant_count\":{count},\"colliding_variants\":{colliding}}}"
    )?;
    Ok((count, colliding))
}

struct LimitedWriter<W> {
    inner: W,
    written: usize,
    limit: usize,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.written) {
            return Err(io::Error::other(format!(
                "output JSON exceeds {} bytes",
                self.limit
            )));
        }
        let written = self.inner.write(bytes)?;
        self.written += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
