//! Shapefile discovery and reading (geometry + DBF attributes + .prj CRS).
//!
//! Coordinates pass through verbatim: no reprojection is performed.

use std::path::Path;

use serde_json::{Map, Value};
use walkdir::WalkDir;

use crate::error::ApiError;

pub struct ShapefileFeature {
    pub geometry: geo_types::Geometry<f64>,
    pub properties: Map<String, Value>,
}

pub struct ShapefileData {
    /// WKT CRS from the .prj sidecar, if present.
    pub crs_wkt: Option<String>,
    pub features: Vec<ShapefileFeature>,
}

/// Find all `*.shp` files (case-insensitive) under `dir`, sorted for
/// deterministic merge order.
pub fn discover(dir: &Path) -> Result<Vec<std::path::PathBuf>, ApiError> {
    let mut paths: Vec<std::path::PathBuf> = WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("shp"))
                .unwrap_or(false)
        })
        .map(|e| e.into_path())
        .collect();
    paths.sort();
    Ok(paths)
}

/// Read a single shapefile: features, properties and optional .prj WKT.
pub fn read(shp_path: &Path) -> Result<ShapefileData, ApiError> {
    let mut reader = shapefile::Reader::from_path(shp_path)
        .map_err(|e| ApiError::BadRequest(format!("cannot open shapefile: {e}")))?;

    let prj_path = shp_path.with_extension("prj");
    let crs_wkt = std::fs::read_to_string(&prj_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut features = Vec::new();
    for result in reader.iter_shapes_and_records() {
        let (shape, record) =
            result.map_err(|e| ApiError::BadRequest(format!("bad shapefile record: {e}")))?;
        let geometry = match geo_types::Geometry::<f64>::try_from(shape) {
            Ok(g) => g,
            Err(_) => continue, // NullShape: skip feature
        };
        features.push(ShapefileFeature {
            geometry,
            properties: record_to_json(&record),
        });
    }

    tracing::debug!(
        path = %shp_path.display(),
        features = features.len(),
        crs = crs_wkt.is_some(),
        "read shapefile"
    );

    Ok(ShapefileData { crs_wkt, features })
}

/// Map a DBF record to a JSON object. NULL/missing values become JSON null.
/// Keys are sorted for deterministic output (the inner store is a HashMap).
fn record_to_json(record: &dbase::Record) -> Map<String, Value> {
    let mut pairs: Vec<(String, Value)> = record
        .as_ref()
        .iter()
        .map(|(name, value)| (name.clone(), field_value_to_json(value)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs.into_iter().collect()
}

fn field_value_to_json(value: &dbase::FieldValue) -> Value {
    use dbase::FieldValue::*;
    match value {
        Character(Some(s)) => Value::String(s.clone()),
        Numeric(Some(n)) => json_f64(*n),
        Float(Some(f)) => json_f64(*f as f64),
        Logical(Some(b)) => Value::Bool(*b),
        Date(Some(d)) => Value::String(format!(
            "{:04}-{:02}-{:02}",
            d.year(),
            d.month(),
            d.day()
        )),
        DateTime(dt) => {
            let d = dt.date();
            let t = dt.time();
            Value::String(format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                d.year(),
                d.month(),
                d.day(),
                t.hours(),
                t.minutes(),
                t.seconds()
            ))
        }
        Integer(i) => Value::Number((*i).into()),
        Currency(c) => json_f64(*c),
        Double(d) => json_f64(*d),
        Memo(s) => Value::String(s.clone()),
        _ => Value::Null, // all-padded/blank values
    }
}

fn json_f64(n: f64) -> Value {
    serde_json::Number::from_f64(n)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}
