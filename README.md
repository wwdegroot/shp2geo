# Shapefile to geo formats
Small http api service that converts shapefile uploaded as a zip to either geojson or flatgeobuf.

## Requirements
- In crs to out crs, no transformation or reprojection.
- url query param called `f` outputs either json `f=json`  or flatgeobuf `f=fgb`
- minimal docker container image, for example use debian static.

## Tech stack
- rust axum web server
- flatgeobuf
- geojson
