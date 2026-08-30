# Plan: Shapefile → GeoJSON / FlatGeobuf REST API

## Goal

A small HTTP service that accepts a zipped shapefile and returns it converted
to GeoJSON or FlatGeobuf. No reprojection: coordinates pass through verbatim;
source CRS metadata is carried into the output.

## Confirmed decisions

- **CRS handling**: pass-through + metadata. No transformation/reprojection.
  - FGB: keep the source CRS (WKT from `.prj`) in the FGB header.
  - GeoJSON: embed a non-standard `crs` member (pre-RFC 7946 style) holding the
    source CRS since RFC 7946 assumes CRS84.
- **Multiple shapefiles in the zip**: convert all of them and merge all
  features into a single output dataset.
- **Upload**: `multipart/form-data` with a `file` field containing the zip.
- **Endpoints**: `POST /convert` + `GET /healthz`.

## API spec

### `GET /healthz`

- `200 OK` with a tiny JSON body (e.g. `{"status":"ok"}`). No dependencies, no
  I/O — for container orchestration.

### `POST /convert?f=json|fgb`

- Query param `f`:
  - `json` → GeoJSON (`application/geo+json`)
  - `fgb` → FlatGeobuf (`application/octet-stream`)
  - omitted/unknown → default `json`; unknown value → `400` with JSON error.
- Body: `multipart/form-data`, field `file` = the `.zip` archive.
- Success: `200` with the converted payload in the response body.
- Errors: JSON `{"error": "<message>"}` with appropriate status:
  - `400` — missing/invalid file field, not a zip, no `.shp` found,
    unreadable shapefile, missing required sidecar files (`.dbf`, `.shx`).
  - `413` — upload too large (sane cap, e.g. 512 MB, configurable).
  - `405` / `404` — wrong method / unknown route.

## Tech stack

| Concern            | Crate (planned)                              |
|--------------------|----------------------------------------------|
| HTTP framework     | `axum` + `tokio`                             |
| Multipart upload   | `axum`'s `multipart` feature                 |
| Zip extraction     | `zip` crate                                  |
| Shapefile reading  | `shapefile` crate (shp/dbf/shx reader)       |
| GeoJSON output     | `geojson` crate                              |
| FlatGeobuf output  | `flatgeobuf` crate (geozero-based writer)    |
| CRS metadata       | read `.prj` WKT, pass verbatim into FGB crs / GeoJSON `crs` member |
| Errors / logging   | `thiserror`, `tracing` + `tracing-subscriber`|

## Architecture

```
src/
  main.rs        # thin binary: tokio runtime, tracing init, graceful serve
  lib.rs         # library root exposing modules (used by integration tests)
  routes.rs      # GET /healthz, POST /convert handlers, 404/405 fallbacks
  upload.rs      # multipart parsing + zip-slip-safe extraction into TempDir
  shapefile.rs   # discover .shp files, read records+fields+prj
  convert/
    mod.rs       # enum OutputFormat { Json, Fgb }, from query param
    geojson.rs   # features (with source crs member) → FeatureCollection
    fgb.rs       # features (with crs WKT) → FlatGeobuf bytes
  error.rs       # ApiError enum → IntoResponse (status + JSON body)
  limits.rs      # PORT / MAX_UPLOAD_BYTES / MAX_EXTRACTED_BYTES via env vars
```

Conversion pipeline (in memory, no reprojection):

1. Extract uploaded zip to a temp dir (zip-slip guarded, symlinks/encryption
   rejected, per-entry + total decompressed size caps).
2. Find all `*.shp` (sorted); if none → 400.
3. For each shapefile: read geometry records + DBF attributes; read `.prj`
   WKT if present (per-shapefile; first one found wins for merged output).
4. Convert geometries to `geo` types (X/Y pass through bit-for-bit; Z/M and
   NullShape are dropped/skipped).
5. Serialize to the requested format:
   - GeoJSON: `FeatureCollection`, each `Feature` has `geometry` + `properties`
     (DBF fields) + collection-level non-standard `crs` member.
   - FGB: `FgbWriter` with header `crs: WKT` and a pre-declared union schema of
     all attribute columns (so heterogeneous DBF schemas merge correctly);
     geometry type `Unknown` to allow mixed geometries. Note: FGB output is
     Hilbert-sorted, so feature order is not preserved.
6. Return bytes with correct `Content-Type`.

## Implementation steps

### Milestone 1 — Skeleton
- [x] `cargo init`, add deps (axum, tokio, tracing, thiserror, serde, serde_json).
- [x] `GET /healthz` returning `{"status":"ok"}`.
- [x] `error.rs` with `ApiError` → JSON responses with status codes.
- [x] Config via env vars: `PORT`, `MAX_UPLOAD_BYTES` (defaults 3000, 25 MB).

### Milestone 2 — Upload handling
- [x] `POST /convert` with `multipart` feature; extract `file` field.
- [x] Enforce upload size cap → `413`.
- [x] Validate `.zip`, extract to temp dir (zip-slip safe), clean up on drop.

### Milestone 3 — Shapefile reading
- [x] Discover all `.shp` in the archive; error if none (`400`).
- [x] Read features: geometry (`shapefile` crate) + DBF properties as JSON map.
- [x] Read `.prj` WKT per shapefile; optional (absent → no crs member).

### Milestone 4 — GeoJSON output
- [x] `f=json` (default): build `geojson::FeatureCollection`.
- [x] Insert non-standard top-level `"crs": {"type":"name","properties":{"name":"<WKT>"}}` when a `.prj` exists.
- [x] Response `Content-Type: application/geo+json`.

### Milestone 5 — FlatGeobuf output
- [x] `f=fgb`: build FGB via `flatgeobuf` writer (geozero API), set header
      `crs` to the `.prj` WKT.
- [x] Response `Content-Type: application/octet-stream`.

### Milestone 6 — Hardening & polish
- [x] Proper 404/405 responses; unknown `f` → 400.
- [x] `tracing` request logging; error logging with context.
- [x] Unit tests: format selection, error mapping.
- [x] Integration test: fixture zip (point/polygon shapefile w/ `.prj`) →
      round-trip through both formats, assert feature counts, properties,
      crs presence, untouched coordinates. (FGB verified by reading the
      output back with `FgbReader` — feature order is Hilbert-sorted.)

### Milestone 7 — Docker
- [x] Multi-stage `Dockerfile`:
      - Stage 1: `rust:1-slim` + musl-tools → fully static binary.
      - Stage 2: `gcr.io/distroless/static-debian12:nonroot` runtime image,
        copy binary, non-root `USER`, `EXPOSE`. Note: distroless has no shell,
        so no `HEALTHCHECK` — use orchestrator HTTP probes on `/healthz`.


## Testing checklist (acceptance)

Covered by `cargo test` (6 unit + 9 integration tests) unless noted:

- [x] `f=json` → valid GeoJSON with crs member, `application/geo+json`.
- [x] `f=fgb` → valid FGB with header crs WKT, verified by round-trip read
      via `FgbReader`/geozero (manual `ogrinfo` check optional).
- [x] Zip with two shapefiles → single merged output.
- [x] Zip without `.shp` → 400 JSON error.
- [x] Corrupt/renamed-zip upload → 400.
- [x] Oversized upload → 413.
- [x] `GET /healthz` → 200.
- [x] Coordinates identical between input shapefile and both outputs
      (bit-for-bit CRS pass-through, no reprojection).
