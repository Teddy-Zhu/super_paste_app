use std::{fmt, io::Write, sync::Arc};
use base64::{display::Base64Display, engine::general_purpose::STANDARD};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::{Serialize, Serializer};
use tokio_util::sync::CancellationToken;
use crate::{Error, Result, images::ImageService};

pub(super) struct Base64Bytes<'a>(pub &'a [u8]);
impl Serialize for Base64Bytes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(&Base64Display::new(self.0, &STANDARD))
    }
}
pub(super) struct DataUrl<'a> { pub mime: &'a str, pub bytes: &'a [u8] }
impl fmt::Display for DataUrl<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "data:{};base64,{}", self.mime, Base64Display::new(self.bytes, &STANDARD))
    }
}
impl Serialize for DataUrl<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> { serializer.collect_str(self) }
}
pub(super) async fn read_response(
    response: reqwest::Response, images: &Arc<ImageService>, cancel: &CancellationToken,
    maximum: u64, too_large: &'static str, request_error: impl Fn(reqwest::Error) -> Error,
) -> Result<Bytes> {
    if response.content_length().is_some_and(|size| size > maximum) { return Err(Error::ResourceLimit(too_large.into())); }
    let mut writer = images.scratch_writer()?;
    let mut length = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = tokio::select! {
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        next = stream.next() => next,
    } {
        let chunk = chunk.map_err(&request_error)?;
        length = length.saturating_add(chunk.len() as u64);
        if length > maximum { return Err(Error::ResourceLimit(too_large.into())); }
        writer.write_all(&chunk)?;
    }
    writer.into_bytes()
}
