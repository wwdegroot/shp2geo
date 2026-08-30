//! FlatGeobuf serialization via the geozero writer API.
//!
//! The source CRS WKT is stored verbatim in the FGB header. A union schema of
//! all attribute columns is declared up front so shapefiles with different
//! DBF schemas merge correctly. Coordinates pass through unchanged.

use std::collections::BTreeSet;

use flatgeobuf::{
    ColumnType, FgbCrs, FgbWriter, FgbWriterOptions, GeometryType,
};
use geozero::{ColumnValue, PropertyProcessor};
use serde_json::Value;

use crate::error::ApiError;
use crate::shapefile::ShapefileData;

/// Ordered union of all attribute names across all merged shapefiles.
fn union_schema(shapefiles: &[ShapefileData]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for sf in shapefiles {
        for f in &sf.features {
            for name in f.properties.keys() {
                if names.insert(name.clone()) {
                    seen.push(name.clone());
                }
            }
        }
    }
    seen
}

fn infer_column_type(shapefiles: &[ShapefileData], name: &str) -> ColumnType {
    let mut saw_bool = false;
    let mut saw_int = false;
    let mut saw_float = false;
    for sf in shapefiles {
        for f in &sf.features {
            match f.properties.get(name) {
                Some(Value::Bool(_)) => saw_bool = true,
                Some(Value::Number(n)) => {
                    if n.is_i64() || n.is_u64() {
                        saw_int = true;
                    } else {
                        saw_float = true;
                    }
                }
                Some(Value::String(_)) | Some(Value::Array(_)) | Some(Value::Object(_)) => {
                    return ColumnType::String
                }
                _ => {}
            }
        }
    }
    if saw_bool && (saw_int || saw_float) {
        return ColumnType::String; // mixed types: stringify
    }
    if saw_float {
        ColumnType::Double
    } else if saw_int {
        ColumnType::Long
    } else if saw_bool {
        ColumnType::Bool
    } else {
        ColumnType::String // all-null column
    }
}

fn column_value(v: &Value, col_type: ColumnType) -> ColumnValue<'_> {
    match col_type {
        ColumnType::Bool => ColumnValue::Bool(matches!(v, Value::Bool(true))),
        ColumnType::Long => ColumnValue::Long(match v {
            Value::Number(n) => n.as_f64().unwrap_or_default() as i64,
            Value::Bool(b) => *b as i64,
            _ => 0,
        }),
        ColumnType::Double => ColumnValue::Double(match v {
            Value::Number(n) => n.as_f64().unwrap_or_default(),
            Value::Bool(b) => *b as i64 as f64,
            _ => f64::NAN,
        }),
        _ => ColumnValue::String(match v {
            Value::String(s) => s.as_str(),
            other => other.to_string().leak(), // rare: mixed-type fallback
        }),
    }
}

pub fn to_fgb(
    shapefiles: &[ShapefileData],
    crs_wkt: Option<&str>,
) -> Result<Vec<u8>, ApiError> {
    let schema = union_schema(shapefiles);
    let mut options = FgbWriterOptions {
        write_index: true,
        detect_type: false,
        promote_to_multi: false,
        ..Default::default()
    };
    if let Some(wkt) = crs_wkt {
        options.crs = FgbCrs {
            wkt: Some(wkt),
            ..Default::default()
        };
    }

    let mut fgb = FgbWriter::create_with_options(
        "converted",
        GeometryType::Unknown, // mixed geometries allowed by pass-through
        options,
    )
    .map_err(|e| ApiError::Internal(format!("fgb init: {e}")))?;

    // Pre-declare the union schema so features with differing DBF schemas
    // can be written into matching column indexes.
    let mut col_types = Vec::with_capacity(schema.len());
    for (idx, name) in schema.iter().enumerate() {
        let col_type = infer_column_type(shapefiles, name);
        col_types.push(col_type);
        let owned = name.clone();
        fgb.add_column(&owned, col_type, |_fbb, col| {
            let _ = idx;
            col.nullable = true;
        });
    }

    for sf in shapefiles {
        for feature in &sf.features {
            let properties = &feature.properties;
            // add_feature_geom takes an owned GeozeroGeometry
            let geometry = feature.geometry.clone();
            let schema = &schema;
            let col_types = &col_types;
            fgb.add_feature_geom(geometry, |feat| {
                for (idx, name) in schema.iter().enumerate() {
                    if let Some(v) = properties.get(name) {
                        if !v.is_null() {
                            let _ = feat.property(idx, name, &column_value(v, col_types[idx]));
                        }
                    }
                }
            })
            .map_err(|e| ApiError::Internal(format!("fgb feature: {e}")))?;
        }
    }

    let mut out = Vec::new();
    fgb.write(&mut out)
        .map_err(|e| ApiError::Internal(format!("fgb write: {e}")))?;
    Ok(out)
}
