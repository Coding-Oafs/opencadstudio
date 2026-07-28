//! Browser-backed copies of recently opened drawings.
//!
//! A web file picker exposes bytes and a display name, not a reusable native
//! path. Keep the last-opened copy in the origin-private file system (OPFS) so
//! the Start page can reopen it without asking the user to pick it again.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

const RECENT_DIRECTORY: &str = "opencadstudio-recent";

pub async fn store(name: &str, bytes: &[u8]) -> Result<(), String> {
    let directory = recent_directory(true).await?;
    let options = web_sys::FileSystemGetFileOptions::new();
    options.set_create(true);
    let handle = JsFuture::from(directory.get_file_handle_with_options(&cache_key(name), &options))
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
    let handle = JsFuture::from(directory.get_file_handle(&cache_key(name)))
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::FileSystemFileHandle>()
        .map_err(js_error)?;
    let file = JsFuture::from(handle.get_file())
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::File>()
        .map_err(js_error)?;
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(js_error)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

pub async fn remove(name: &str) -> Result<(), String> {
    let directory = recent_directory(false).await?;
    JsFuture::from(directory.remove_entry(&cache_key(name)))
        .await
        .map_err(js_error)?;
    Ok(())
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
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}.cad")
}

fn js_error(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser storage error: {value:?}"))
}
