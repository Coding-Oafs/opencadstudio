use std::io::Cursor;

use acadrust::io::dwg::DwgReader;
use acadrust::DxfReader;
use js_sys::{Function, Uint8Array};
use wasm_bindgen::prelude::*;

/// Parse DWG/DXF on a dedicated browser worker and return a compact serialized
/// document. The main wasm instance only deserializes and installs it, so the
/// expensive bit/handle/object decode never occupies the browser UI thread.
#[wasm_bindgen]
pub fn parse_document(
    name: String,
    bytes: Uint8Array,
    report_stage: &Function,
) -> Result<Uint8Array, JsValue> {
    console_error_panic_hook::set_once();
    report_stage.call1(&JsValue::NULL, &JsValue::from_str("copy input"))?;
    let bytes = bytes.to_vec();
    report_stage.call1(&JsValue::NULL, &JsValue::from_str("parse document"))?;
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    let document = match ext.as_str() {
        "dwg" => DwgReader::from_stream(Cursor::new(bytes))
            .read()
            .map_err(|error| JsValue::from_str(&error.to_string()))?,
        "dxf" => DxfReader::from_reader(Cursor::new(bytes))
            .map_err(|error| JsValue::from_str(&error.to_string()))?
            .read()
            .map_err(|error| JsValue::from_str(&error.to_string()))?,
        _ => {
            return Err(JsValue::from_str(&format!(
                "Unsupported file format: .{ext}"
            )))
        }
    };
    report_stage.call1(&JsValue::NULL, &JsValue::from_str("serialize document"))?;
    let encoded =
        bincode::serialize(&document).map_err(|error| JsValue::from_str(&error.to_string()))?;
    report_stage.call1(&JsValue::NULL, &JsValue::from_str("copy output"))?;
    Ok(Uint8Array::from(encoded.as_slice()))
}
