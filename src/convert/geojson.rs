//! GeoJSON serialization.
//!
//! RFC 7946 has no CRS support (it assumes CRS84), so a non-standard,
//! pre-RFC-7946 style `crs` member is embedded when the source has a .prj.

use serde_json::{json, Map};

use crate::error::ApiError;
use crate::shapefile::ShapefileData;

pub fn to_geojson(
    shapefiles: &[ShapefileData],
    crs_wkt: Option<&str>,
) -> Result<Vec<u8>, ApiError> {
    let features = shapefiles
        .iter()
        .flat_map(|s| s.features.iter())
        .map(|f| {
            let geometry = geojson::Geometry::new(geojson::GeometryValue::from(&f.geometry));
            geojson::Feature {
                bbox: None,
                geometry: Some(geometry),
                id: None,
                properties: Some(f.properties.clone()),
                foreign_members: None,
            }
        })
        .collect();

    let mut fc = geojson::FeatureCollection {
        bbox: None,
        features,
        foreign_members: None,
    };

    if let Some(wkt) = crs_wkt {
        let mut crs = Map::new();
        crs.insert("type".into(), json!("name"));
        crs.insert("properties".into(), json!({ "name": wkt }));
        fc.foreign_members = Some(Map::from_iter([("crs".to_string(), json!(crs))]));
    }

    serde_json::to_vec(&fc).map_err(|e| ApiError::Internal(e.to_string()))
}
