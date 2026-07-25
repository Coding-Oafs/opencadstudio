use std::cell::RefCell;
use std::rc::Rc;

use acadrust::CadDocument;
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{ErrorEvent, MessageEvent, Worker, WorkerOptions, WorkerType};

pub(super) async fn parse_document(name: &str, bytes: Vec<u8>) -> Result<CadDocument, String> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options("ocs-parse-worker.js", &options).map_err(js_error)?;

    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let message_sender = sender.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let data = event.data();
        let ok = Reflect::get(&data, &JsValue::from_str("ok"))
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let result = if ok {
            Reflect::get(&data, &JsValue::from_str("data"))
                .map_err(js_error)
                .and_then(|value| {
                    let bytes = Uint8Array::new(&value).to_vec();
                    bincode::deserialize(&bytes).map_err(|error| error.to_string())
                })
        } else {
            Err(Reflect::get(&data, &JsValue::from_str("error"))
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_else(|| "CAD parser worker failed".to_string()))
        };
        if let Some(sender) = message_sender.borrow_mut().take() {
            let _ = sender.send(result);
        }
    });
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let error_sender = sender;
    let on_error = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
        if let Some(sender) = error_sender.borrow_mut().take() {
            let _ = sender.send(Err(event.message()));
        }
    });
    worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    let payload = Object::new();
    Reflect::set(
        &payload,
        &JsValue::from_str("name"),
        &JsValue::from_str(name),
    )
    .map_err(js_error)?;
    let input = Uint8Array::from(bytes.as_slice());
    Reflect::set(&payload, &JsValue::from_str("bytes"), &input.buffer()).map_err(js_error)?;
    let transfer = Array::new();
    transfer.push(&input.buffer());
    worker
        .post_message_with_transfer(&payload, &transfer)
        .map_err(js_error)?;

    let result = receiver
        .await
        .map_err(|_| "CAD parser worker closed without a result".to_string())?;
    worker.terminate();
    result
}

fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser worker error: {value:?}"))
}
