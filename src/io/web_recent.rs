//! Browser-backed copies of recently opened drawings.
//!
//! A web file picker exposes bytes and a display name, not a reusable native
//! path. Keep the last-opened copy in the origin-private file system (OPFS) so
//! the Start page can reopen it without asking the user to pick it again.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

const RECENT_DIRECTORY: &str = "opencadstudio-recent";
const THUMBNAIL_MAGIC: &[u8; 4] = b"OCST";
const THUMBNAIL_MAX_DIM: u32 = 256;

pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub async fn store(name: &str, bytes: &[u8]) -> Result<(), String> {
    let directory = recent_directory(true).await?;
    write_entry(&directory, &cache_key(name), bytes).await?;

    if let Some(image) = dwg_thumbnailer::extract_bytes(bytes, THUMBNAIL_MAX_DIM) {
        let thumbnail = encode_thumbnail(image);
        // The drawing copy is the durable part of a recent entry. Thumbnail
        // caching must not make an otherwise successful open/save fail under
        // browser quota pressure.
        let _ = write_entry(&directory, &thumbnail_key(name), &thumbnail).await;
    } else {
        // An overwrite may replace a drawing that had a preview with one that
        // does not. Do not leave the old image attached to the new file.
        let _ = JsFuture::from(directory.remove_entry(&thumbnail_key(name))).await;
    }
    Ok(())
}

async fn write_entry(
    directory: &web_sys::FileSystemDirectoryHandle,
    key: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let options = web_sys::FileSystemGetFileOptions::new();
    options.set_create(true);
    let handle = JsFuture::from(directory.get_file_handle_with_options(key, &options))
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::FileSystemFileHandle>()
        .map_err(js_error)?;
    let writable = JsFuture::from(handle.create_writable())
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::FileSystemWritableFileStream>()
        .map_err(js_error)?;
    let write = writable.write_with_u8_array(bytes).map_err(js_error)?;
    JsFuture::from(write).await.map_err(js_error)?;
    JsFuture::from(writable.close()).await.map_err(js_error)?;
    Ok(())
}

pub async fn read(name: &str) -> Result<Vec<u8>, String> {
    let directory = recent_directory(false).await?;
    read_entry(&directory, &cache_key(name)).await
}

async fn read_entry(
    directory: &web_sys::FileSystemDirectoryHandle,
    key: &str,
) -> Result<Vec<u8>, String> {
    let file = get_file(directory, key).await?;
    read_blob(file.as_ref()).await
}

async fn get_file(
    directory: &web_sys::FileSystemDirectoryHandle,
    key: &str,
) -> Result<web_sys::File, String> {
    let handle = JsFuture::from(directory.get_file_handle(key))
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::FileSystemFileHandle>()
        .map_err(js_error)?;
    JsFuture::from(handle.get_file())
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::File>()
        .map_err(js_error)
}

async fn read_blob(blob: &web_sys::Blob) -> Result<Vec<u8>, String> {
    let buffer = JsFuture::from(blob.array_buffer())
        .await
        .map_err(js_error)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Load a cached preview. Records created before thumbnail sidecars existed are
/// migrated by slicing only the DWG header and preview container from the OPFS
/// file; the potentially large drawing body is never copied or parsed.
pub async fn read_thumbnail(name: &str) -> Result<Option<Thumbnail>, String> {
    let directory = recent_directory(false).await?;
    if let Ok(bytes) = read_entry(&directory, &thumbnail_key(name)).await {
        if let Some(thumbnail) = decode_thumbnail(bytes) {
            return Ok(Some(thumbnail));
        }
    }

    let file = get_file(&directory, &cache_key(name)).await?;
    let Some(image) = extract_embedded_thumbnail(&file).await? else {
        return Ok(None);
    };
    let encoded = encode_thumbnail(image);
    let thumbnail = decode_thumbnail(encoded.clone())
        .ok_or_else(|| "generated recent thumbnail is invalid".to_string())?;
    // Migration caching is best-effort: a readable drawing should still show
    // its preview even if quota pressure prevents writing the sidecar.
    let _ = write_entry(&directory, &thumbnail_key(name), &encoded).await;
    Ok(Some(thumbnail))
}

pub async fn remove(name: &str) -> Result<(), String> {
    let directory = recent_directory(false).await?;
    let drawing = JsFuture::from(directory.remove_entry(&cache_key(name)))
        .await
        .map_err(js_error);
    let _ = JsFuture::from(directory.remove_entry(&thumbnail_key(name))).await;
    drawing.map(|_| ())
}

async fn recent_directory(create: bool) -> Result<web_sys::FileSystemDirectoryHandle, String> {
    let window = web_sys::window().ok_or_else(|| "browser window unavailable".to_string())?;
    let root = JsFuture::from(window.navigator().storage().get_directory())
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::FileSystemDirectoryHandle>()
        .map_err(js_error)?;
    let options = web_sys::FileSystemGetDirectoryOptions::new();
    options.set_create(create);
    JsFuture::from(root.get_directory_handle_with_options(RECENT_DIRECTORY, &options))
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::FileSystemDirectoryHandle>()
        .map_err(js_error)
}

/// Stable, short OPFS entry name. The original display name remains in
/// `AppConfig::recent`; only the browser-private cache uses this key.
fn cache_key(name: &str) -> String {
    format!("{}.cad", name_hash(name))
}

fn thumbnail_key(name: &str) -> String {
    format!("{}.thumb", name_hash(name))
}

fn name_hash(name: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn encode_thumbnail(image: dwg_thumbnailer::RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(12 + image.as_raw().len());
    bytes.extend_from_slice(THUMBNAIL_MAGIC);
    bytes.extend_from_slice(&image.width().to_le_bytes());
    bytes.extend_from_slice(&image.height().to_le_bytes());
    bytes.extend_from_slice(image.as_raw());
    bytes
}

fn decode_thumbnail(bytes: Vec<u8>) -> Option<Thumbnail> {
    if bytes.get(..4)? != THUMBNAIL_MAGIC {
        return None;
    }
    let width = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?);
    let height = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
    let expected = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)?;
    if width == 0
        || height == 0
        || width > THUMBNAIL_MAX_DIM
        || height > THUMBNAIL_MAX_DIM
        || bytes.len() != 12 + expected
    {
        return None;
    }
    Some(Thumbnail {
        width,
        height,
        rgba: bytes[12..].to_vec(),
    })
}

async fn extract_embedded_thumbnail(
    file: &web_sys::File,
) -> Result<Option<dwg_thumbnailer::RgbaImage>, String> {
    let header = read_file_range(file, 0, 0x11).await?;
    if header.get(..2) != Some(b"AC") {
        return Ok(None);
    }
    let Some(offset) = header
        .get(0x0D..0x11)
        .and_then(|bytes| bytes.try_into().ok())
        .map(i32::from_le_bytes)
        .filter(|offset| *offset > 0)
    else {
        return Ok(None);
    };
    let offset = offset as u64;
    let container_header = read_file_range(file, offset, offset + 20).await?;
    let Some(overall) = container_header
        .get(16..20)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .map(u64::from)
        .filter(|size| *size > 0 && *size <= 64 * 1024 * 1024)
    else {
        return Ok(None);
    };
    let Some(end) = offset
        .checked_add(36)
        .and_then(|end| end.checked_add(overall))
    else {
        return Ok(None);
    };
    let container = read_file_range(file, offset, end).await?;
    Ok(dwg_thumbnailer::extract_container(
        &container,
        offset,
        THUMBNAIL_MAX_DIM,
    ))
}

async fn read_file_range(file: &web_sys::File, start: u64, end: u64) -> Result<Vec<u8>, String> {
    let blob: &web_sys::Blob = file.as_ref();
    let slice = blob
        .slice_with_f64_and_f64(start as f64, end as f64)
        .map_err(js_error)?;
    read_blob(&slice).await
}

fn js_error(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser storage error: {value:?}"))
}
