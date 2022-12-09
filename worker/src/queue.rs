use std::marker::PhantomData;

use crate::{env::EnvBinding, Date, Error, Result};
use js_sys::Array;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use worker_sys::{MessageBatch as MessageBatchSys, Queue as EdgeQueue};

static BODY_KEY_STR: &str = "body";
static ID_KEY_STR: &str = "id";
static TIMESTAMP_KEY_STR: &str = "timestamp";

/// # Examples
///```no_run
/// #[event(queue)]
/// pub async fn queue(message_batch: MessageBatch<MyType>, _env: Env, _ctx: Context) -> Result<()> {
///     for message in message_batch.iter() {
///         let message = message?;
///         console_log!(
///             "Received queue message {:?}, with id {} and timestamp: {}",
///             message.body,
///             message.id,
///             message.timestamp.to_string()
///         );
///     }
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct MessageBatch<T> {
    inner: MessageBatchSys,
    messages: Array,
    data: PhantomData<T>,
    timestamp_key: JsValue,
    body_key: JsValue,
    id_key: JsValue,
}

impl<T> MessageBatch<T> {
    pub fn new(message_batch_sys: MessageBatchSys) -> Self {
        let timestamp_key = JsValue::from_str(TIMESTAMP_KEY_STR);
        let body_key = JsValue::from_str(BODY_KEY_STR);
        let id_key = JsValue::from_str(ID_KEY_STR);
        Self {
            messages: message_batch_sys.messages(),
            inner: message_batch_sys,
            data: PhantomData,
            timestamp_key,
            body_key,
            id_key,
        }
    }
}

pub struct Message<T> {
    pub body: T,
    pub timestamp: Date,
    pub id: String,
}

impl<T> MessageBatch<T> {
    /// The name of the Queue that belongs to this batch.
    pub fn queue(&self) -> String {
        self.inner.queue()
    }

    /// Marks every message to be retried in the next batch.
    pub fn retry_all(&self) {
        self.inner.retry_all();
    }

    /// Iterator that deserializes messages in the message batch. Ordering of messages is not guaranteed.
    pub fn iter(&self) -> MessageIter<'_, T>
    where
        T: for<'de> Deserialize<'de>,
    {
        MessageIter {
            range: 0..self.messages.length(),
            array: &self.messages,
            timestamp_key: &self.timestamp_key,
            body_key: &self.body_key,
            id_key: &self.id_key,
            data: PhantomData,
        }
    }
}

pub struct MessageIter<'a, T>
where
    T: Deserialize<'a>,
{
    range: std::ops::Range<u32>,
    array: &'a Array,
    timestamp_key: &'a JsValue,
    body_key: &'a JsValue,
    id_key: &'a JsValue,
    data: PhantomData<T>,
}

fn parse_message<T>(
    message: &JsValue,
    timestamp_key: &JsValue,
    body_key: &JsValue,
    id_key: &JsValue,
) -> Result<Message<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let js_date = js_sys::Date::from(js_sys::Reflect::get(message, timestamp_key)?);
    let id = js_sys::Reflect::get(message, id_key)?
        .as_string()
        .ok_or(Error::JsError(
            "Invalid message batch. Failed to get id from message.".to_string(),
        ))?;

    let body = serde_wasm_bindgen::from_value(js_sys::Reflect::get(message, body_key)?)?;

    Ok(Message {
        id,
        body,
        timestamp: Date::from(js_date),
    })
}

impl<'a, T> std::iter::Iterator for MessageIter<'a, T>
where
    T: for<'de> Deserialize<'de>,
{
    type Item = Result<Message<T>>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.range.next()?;

        let value = self.array.get(index);

        Some(parse_message(
            &value,
            self.timestamp_key,
            self.body_key,
            self.id_key,
        ))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, T> std::iter::DoubleEndedIterator for MessageIter<'a, T>
where
    T: for<'de> Deserialize<'de>,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        let index = self.range.next_back()?;
        let value = self.array.get(index);

        Some(parse_message(
            &value,
            self.timestamp_key,
            self.body_key,
            self.id_key,
        ))
    }
}

impl<'a, T> std::iter::FusedIterator for MessageIter<'a, T> where T: for<'de> Deserialize<'de> {}

impl<'a, T> std::iter::ExactSizeIterator for MessageIter<'a, T> where T: for<'de> Deserialize<'de> {}

pub struct Queue(EdgeQueue);

impl EnvBinding for Queue {
    const TYPE_NAME: &'static str = "WorkerQueue";
}

impl JsCast for Queue {
    fn instanceof(val: &JsValue) -> bool {
        val.is_instance_of::<Queue>()
    }

    fn unchecked_from_js(val: JsValue) -> Self {
        Self(val.into())
    }

    fn unchecked_from_js_ref(val: &JsValue) -> &Self {
        unsafe { &*(val as *const JsValue as *const Self) }
    }
}

impl From<Queue> for JsValue {
    fn from(queue: Queue) -> Self {
        JsValue::from(queue.0)
    }
}

impl AsRef<JsValue> for Queue {
    fn as_ref(&self) -> &JsValue {
        &self.0
    }
}

impl Queue {
    /// Sends a message to the Queue.
    pub async fn send<T>(&self, message: &T) -> Result<()>
    where
        T: Serialize,
    {
        let js_value = serde_wasm_bindgen::to_value(message)?;
        let fut: JsFuture = self.0.send(js_value).into();

        fut.await.map_err(Error::from)?;
        Ok(())
    }
}
