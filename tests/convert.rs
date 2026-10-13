//! Integration tests: zip a fixture shapefile set, push it through the
//! axum router and verify both output formats, CRS pass-through and errors.

use std::{collections::HashMap, io::Cursor, path::Path, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use flatgeobuf::FallibleStreamingIterator;
use geozero::{FeatureProperties, GeozeroGeometry};
use http_body_util::BodyExt;
use tower::{Service, ServiceExt};

use shapefile_converter::{config::Limits, error::ApiError, routes::router};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

const WKT: &str = r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["Degree",0.0174532925199433]]"#;

fn write_points_shapefile(dir: &Path, name: &str) {
    let shp = dir.join(format!("{name}.shp"));
    let builder = dbase::TableWriterBuilder::new()
        .add_character_field("name".try_into().unwrap(), 50)
        .add_numeric_field("value".try_into().unwrap(), 10, 2);
    let mut writer = shapefile::Writer::from_path(&shp, builder).unwrap();

    let rec = |nm: &str, v: f64| {
        let mut record = dbase::Record::default();
        record.insert(
            "name".to_string(),
            dbase::FieldValue::Character(Some(nm.to_string())),
        );
        record.insert("value".to_string(), dbase::FieldValue::Numeric(Some(v)));
        record
    };

    writer
        .write_shape_and_record(&shapefile::Point::new(1.0, 2.0), &rec("alpha", 1.5))
        .unwrap();
    writer
        .write_shape_and_record(&shapefile::Point::new(3.0, 4.0), &rec("beta", 2.5))
        .unwrap();
    drop(writer); // flushes .shx

    std::fs::write(dir.join(format!("{name}.prj")), WKT).unwrap();
}

fn write_polygon_shapefile(dir: &Path, name: &str) {
    let shp = dir.join(format!("{name}.shp"));
    let builder =
        dbase::TableWriterBuilder::new().add_character_field("label".try_into().unwrap(), 20);
    let mut writer = shapefile::Writer::from_path(&shp, builder).unwrap();

    let polygon = geo_types::Polygon::new(
        geo_types::LineString::from(vec![
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 4.0),
            (0.0, 4.0),
            (0.0, 0.0),
        ]),
        vec![],
    );
    let mut record = dbase::Record::default();
    record.insert(
        "label".to_string(),
        dbase::FieldValue::Character(Some("square".to_string())),
    );
    writer
        .write_shape_and_record(&shapefile::Polygon::from(polygon), &record)
        .unwrap();
    drop(writer);
    // no .prj on purpose: only the points shapefile carries CRS metadata
}

/// Zip every file in `dir` (flat) into a zip archive.
fn zip_dir(dir: &Path) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        for f in files {
            zip.start_file(
                f.file_name().unwrap().to_str().unwrap().to_string(),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            std::io::copy(&mut std::fs::File::open(&f).unwrap(), &mut zip).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn multipart_request(uri: &str, zip: &[u8]) -> Request<Body> {
    let boundary = "TESTBOUNDARY1234";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"data.zip\"\r\n\
             Content-Type: application/zip\r\n\
             \r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(zip);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    Request::builder()
        .method("POST")
        .uri(uri)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap()
}

async fn post_convert(limits: Arc<Limits>, uri: &str, zip: &[u8]) -> (StatusCode, String, Vec<u8>) {
    let app = router(limits);
    let resp = app.oneshot(multipart_request(uri, zip)).await.unwrap();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, content_type, bytes)
}

async fn get(app: &mut Router, path: &str) -> (StatusCode, Vec<u8>) {
    let req = Request::builder().uri(path).body(Body::empty()).unwrap();
    let resp = Service::call(app, req).await.unwrap();
    let status = resp.status();
    let bytes = BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, bytes)
}

/// Read all features of an FGB buffer: (properties, geometry).
fn read_fgb(fgb: &[u8]) -> Vec<(HashMap<String, String>, geo_types::Geometry<f64>)> {
    assert_eq!(&fgb[..4], b"fgb\x03", "missing fgb magic bytes");
    let reader = flatgeobuf::FgbReader::open(Cursor::new(fgb)).unwrap();
    let mut iter = reader.select_all().unwrap();
    let mut out = Vec::new();
    while let Some(feat) = iter.next().unwrap() {
        let mut props: HashMap<String, String> = HashMap::new();
        feat.process_properties(&mut props).unwrap();
        let mut gw = geozero::geo_types::GeoWriter::new();
        feat.process_geom(&mut gw).unwrap();
        out.push((props, gw.take_geometry().unwrap()));
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn healthz_ok() {
    let limits = Arc::new(Limits::default());
    let mut app = router(limits);
    let (status, body) = get(&mut app, "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, br#"{"status":"ok"}"#.to_vec());
}

#[tokio::test]
async fn geojson_merge_crs_and_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    write_points_shapefile(dir.path(), "points");
    // "z" prefix so points sort first (discovery order is path-sorted)
    write_polygon_shapefile(dir.path(), "zareas");
    let zip = zip_dir(dir.path());

    let limits = Arc::new(Limits::default());
    let (status, content_type, body) = post_convert(limits, "/convert?f=json", &zip).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/geo+json");

    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["type"], "FeatureCollection");
    assert_eq!(v["features"].as_array().unwrap().len(), 3);

    // CRS member carried from the points .prj (WGS84 WKT)
    let crs_name = v["crs"]["properties"]["name"].as_str().unwrap();
    assert!(crs_name.contains("WGS_1984"), "crs member: {crs_name}");

    // coordinates pass through bit-for-bit; properties preserved
    let f0 = &v["features"][0];
    assert_eq!(f0["geometry"]["type"], "Point");
    assert_eq!(f0["geometry"]["coordinates"], serde_json::json!([1.0, 2.0]));
    assert_eq!(f0["properties"]["name"], "alpha");
    assert_eq!(f0["properties"]["value"], serde_json::json!(1.5));

    assert_eq!(
        v["features"][1]["geometry"]["coordinates"],
        serde_json::json!([3.0, 4.0])
    );
    assert_eq!(v["features"][1]["properties"]["name"], "beta");

    // polygon shapefile (different schema, no prj) merged into same collection;
    // shapefile polygons become MultiPolygon in geo-types
    let f2 = &v["features"][2];
    assert_eq!(f2["geometry"]["type"], "MultiPolygon");
    assert_eq!(
        f2["geometry"]["coordinates"][0][0][0],
        serde_json::json!([0.0, 0.0])
    );
    assert_eq!(f2["properties"]["label"], "square");
}

#[tokio::test]
async fn fgb_merge_crs_and_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    write_points_shapefile(dir.path(), "points");
    // "z" prefix so points sort first (discovery order is path-sorted)
    write_polygon_shapefile(dir.path(), "zareas");
    let zip = zip_dir(dir.path());

    let limits = Arc::new(Limits::default());
    let (status, content_type, body) = post_convert(limits, "/convert?f=fgb", &zip).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/octet-stream");

    let features = read_fgb(&body);

    // header CRS = .prj WKT
    let reader = flatgeobuf::FgbReader::open(Cursor::new(&body)).unwrap();
    let crs = reader.header().crs().expect("crs must be set");
    assert!(crs.wkt().expect("crs wkt").contains("WGS_1984"));

    // FGB output is Hilbert-sorted, so key features by a property instead of
    // position. Coordinates must be bit-for-bit identical to the input.
    let key_of = |props: &HashMap<String, String>| {
        props
            .get("name")
            .or_else(|| props.get("label"))
            .cloned()
            .unwrap_or_default()
    };
    let get_feature = |key: &str| features.iter().find(|(p, _)| key_of(p) == key).unwrap();
    assert_eq!(features.len(), 3);
    let keys: std::collections::BTreeSet<String> =
        features.iter().map(|(p, _)| key_of(p)).collect();
    assert_eq!(
        keys.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta", "square"]
    );

    let (props, geom) = get_feature("alpha");
    assert_eq!(props["value"], "1.5");
    match geom {
        geo_types::Geometry::Point(p) => {
            assert_eq!((p.x(), p.y()), (1.0, 2.0));
        }
        other => panic!("expected Point, got {other:?}"),
    }

    let (_, geom) = get_feature("beta");
    match geom {
        geo_types::Geometry::Point(p) => assert_eq!((p.x(), p.y()), (3.0, 4.0)),
        other => panic!("expected Point, got {other:?}"),
    }

    let (props, geom) = get_feature("square");
    assert_eq!(props["label"], "square");
    match geom {
        geo_types::Geometry::MultiPolygon(mp) => {
            let first = mp.0[0].exterior().coords().next().unwrap();
            assert_eq!((first.x, first.y), (0.0, 0.0));
        }
        other => panic!("expected MultiPolygon, got {other:?}"),
    }
}

#[tokio::test]
async fn f_defaults_to_json() {
    let dir = tempfile::tempdir().unwrap();
    write_points_shapefile(dir.path(), "points");
    let zip = zip_dir(dir.path());

    let limits = Arc::new(Limits::default());
    let (status, content_type, body) = post_convert(limits, "/convert", &zip).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/geo+json");
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["type"], "FeatureCollection");
}

#[tokio::test]
async fn unknown_format_is_400() {
    let dir = tempfile::tempdir().unwrap();
    write_points_shapefile(dir.path(), "points");
    let zip = zip_dir(dir.path());

    let limits = Arc::new(Limits::default());
    let (status, _, body) = post_convert(limits, "/convert?f=kml", &zip).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("unsupported output format")
    );
}

#[tokio::test]
async fn zip_without_shapefile_is_400() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("readme.txt"), "no shapefiles here").unwrap();
    let zip = zip_dir(dir.path());

    let limits = Arc::new(Limits::default());
    let (status, _, body) = post_convert(limits, "/convert?f=json", &zip).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["error"].as_str().unwrap().contains("no shapefile"));
}

#[tokio::test]
async fn non_zip_upload_is_400() {
    let limits = Arc::new(Limits::default());
    let (status, _, body) = post_convert(limits, "/convert?f=json", b"definitely not a zip").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["error"].as_str().unwrap().contains("invalid zip"));
}

#[tokio::test]
async fn oversized_upload_is_413() {
    let dir = tempfile::tempdir().unwrap();
    write_points_shapefile(dir.path(), "points");
    let zip = zip_dir(dir.path());

    let limits = Arc::new(Limits {
        max_upload_bytes: 10,
        max_extracted_bytes: 1024,
    });
    let (status, _, body) = post_convert(limits, "/convert?f=json", &zip).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["error"].as_str().unwrap().contains("size limit"));
}

#[tokio::test]
async fn unknown_route_is_404() {
    let limits = Arc::new(Limits::default());
    let mut app = router(limits);
    let (status, body) = get(&mut app, "/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["error"].is_string());
}

// silence unused import if ApiError variant access changes later
#[allow(dead_code)]
fn _api_error_is_used(_: ApiError) {}
