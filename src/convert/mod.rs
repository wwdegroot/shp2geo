//! Output format selection and dispatch.

pub mod fgb;
pub mod geojson;

use crate::error::ApiError;
use crate::shapefile::ShapefileData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    GeoJson,
    FlatGeobuf,
}

impl OutputFormat {
    /// `f=json` (default) or `f=fgb`.
    pub fn from_query(value: Option<&str>) -> Result<Option<Self>, ApiError> {
        match value {
            None => Ok(None),
            Some("json") => Ok(Some(OutputFormat::GeoJson)),
            Some("fgb") => Ok(Some(OutputFormat::FlatGeobuf)),
            Some(other) => Err(ApiError::UnsupportedFormat(other.to_string())),
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            OutputFormat::GeoJson => "application/geo+json",
            OutputFormat::FlatGeobuf => "application/octet-stream",
        }
    }
}

/// Convert all shapefiles to the requested output format.
///
/// Returns the serialized bytes and the response content type.
pub fn convert_all(
    shapefiles: Vec<ShapefileData>,
    format: OutputFormat,
) -> Result<(Vec<u8>, &'static str), ApiError> {
    // CRS metadata: take the first .prj WKT found across merged shapefiles.
    let crs_wkt = shapefiles
        .iter()
        .find_map(|s| s.crs_wkt.clone());

    match format {
        OutputFormat::GeoJson => {
            let body = geojson::to_geojson(&shapefiles, crs_wkt.as_deref())?;
            Ok((body, OutputFormat::GeoJson.content_type()))
        }
        OutputFormat::FlatGeobuf => {
            let body = fgb::to_fgb(&shapefiles, crs_wkt.as_deref())?;
            Ok((body, OutputFormat::FlatGeobuf.content_type()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_selection() {
        assert_eq!(OutputFormat::from_query(None).unwrap(), None);
        assert_eq!(
            OutputFormat::from_query(Some("json")).unwrap(),
            Some(OutputFormat::GeoJson)
        );
        assert_eq!(
            OutputFormat::from_query(Some("fgb")).unwrap(),
            Some(OutputFormat::FlatGeobuf)
        );
        assert!(matches!(
            OutputFormat::from_query(Some("kml")),
            Err(ApiError::UnsupportedFormat(_))
        ));
    }
}
