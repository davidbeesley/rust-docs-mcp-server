use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, Stdin, Stdout};

/// A simple wrapper combining Tokio's Stdin and Stdout
/// to implement both AsyncRead and AsyncWrite for rmcp transport.
#[derive(Debug)] // Added Debug derive
pub struct StdioTransport {
    // Using concrete types Stdin/Stdout as returned by rmcp::transport::io::stdio
    pub reader: Stdin,
    pub writer: Stdout,
}

// Implement AsyncRead by delegating to the inner reader (Stdin)
impl AsyncRead for StdioTransport {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Safety: Accessing fields of a Pin<&mut Self> requires care,
        // but Stdin is Unpin, so projecting to it is safe.
        let this = self.get_mut();
        Pin::new(&mut this.reader).poll_read(cx, buf)
    }
}

// Implement AsyncWrite by delegating to the inner writer (Stdout)
impl AsyncWrite for StdioTransport {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        // Safety: Stdout is Unpin.
        let this = self.get_mut();
        Pin::new(&mut this.writer).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        // Safety: Stdout is Unpin.
        let this = self.get_mut();
        Pin::new(&mut this.writer).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        // Safety: Stdout is Unpin.
        let this = self.get_mut();
        Pin::new(&mut this.writer).poll_shutdown(cx)
    }
}