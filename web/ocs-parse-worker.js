import init, { parse_document, sha256_document } from "./worker_pkg/ocs_web_worker.js?v=3";

const ready = init(
  new URL("./worker_pkg/ocs_web_worker_bg.wasm?v=3", import.meta.url),
);

self.onmessage = async ({ data }) => {
  let stage = "initialize worker";
  try {
    await ready;
    if (data.action === "hash") {
      const digest = sha256_document(new Uint8Array(data.bytes));
      self.postMessage({ ok: true, digest });
      return;
    }
    const encoded = parse_document(
      data.name,
      new Uint8Array(data.bytes),
      data.recoveryMode === true,
      data.initialError || "",
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
      error: `${stage}: ${error instanceof Error ? error.message : String(error)}`,
    });
  }
};
