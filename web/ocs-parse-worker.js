import init, { parse_document } from "./worker_pkg/ocs_web_worker.js";

const ready = init();

self.onmessage = async ({ data }) => {
  try {
    await ready;
    const encoded = parse_document(data.name, new Uint8Array(data.bytes));
    // wasm-bindgen returns a view into WebAssembly.Memory. Copy to a standalone
    // ArrayBuffer before transferring it, otherwise the worker's wasm memory
    // itself would be detached.
    const transferable = encoded.slice();
    self.postMessage({ ok: true, data: transferable.buffer }, [transferable.buffer]);
  } catch (error) {
    self.postMessage({
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
};
