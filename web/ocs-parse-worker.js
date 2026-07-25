import init, { parse_document } from "./worker_pkg/ocs_web_worker.js";

const ready = init();

self.onmessage = async ({ data }) => {
  let stage = "initialize worker";
  try {
    await ready;
    const encoded = parse_document(
      data.name,
      new Uint8Array(data.bytes),
      (next) => {
        stage = next;
      },
    );
    // wasm-bindgen returns a view into WebAssembly.Memory. Copy to a standalone
    // ArrayBuffer before transferring it, otherwise the worker's wasm memory
    // itself would be detached.
    const transferable = encoded.slice();
    self.postMessage({ ok: true, data: transferable.buffer }, [transferable.buffer]);
  } catch (error) {
    self.postMessage({
      ok: false,
      error: `${stage}: ${
        error instanceof Error ? error.stack || error.message : String(error)
      }`,
    });
  }
};
