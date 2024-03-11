use std::fs::File;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::io::Read;

use actix_web::web::Bytes;
use futures::stream::Stream;

pub struct FileStreamer(pub File);

// Implement the Stream trait for FileStream
impl Stream for FileStreamer {
  type Item = Result<Bytes, std::io::Error>;

  fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut buffer = [0; 1024];
    match self.0.read(&mut buffer) {
      Ok(0) => Poll::Ready(None), // End of file
      Ok(n) => Poll::Ready(Some(Ok(Bytes::copy_from_slice(&buffer[..n])))),
      Err(e) => Poll::Ready(Some(Err(e))),
    }
  }
}
