use crate::config::FramingMode;
use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt};

fn io_err(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.into())
}

/// De-frames a TCP syslog stream per RFC 6587: either octet-counting
/// (`"<len> <message>"`, len is the exact byte count of the message that
/// follows the single space) or non-transparent framing (LF-delimited,
/// the historical default). In `Auto` mode the framing is detected once,
/// from the first byte of the connection: an ASCII digit means
/// octet-counting, anything else means non-transparent -- a trailing `\n`
/// on an octet-counted frame would otherwise desync this parser, which is
/// exactly the bug octet-counting framing exists to avoid.
pub struct FrameReader<R> {
    inner: R,
    buf: BytesMut,
    mode: FramingMode,
    max_message_size: usize,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(inner: R, mode: FramingMode, max_message_size: usize) -> Self {
        Self {
            inner,
            buf: BytesMut::with_capacity(4096),
            mode,
            max_message_size,
        }
    }

    pub async fn next_frame(&mut self) -> std::io::Result<Option<Bytes>> {
        loop {
            if self.mode == FramingMode::Auto {
                if let Some(&b) = self.buf.first() {
                    self.mode = if b.is_ascii_digit() {
                        FramingMode::OctetCounting
                    } else {
                        FramingMode::NonTransparent
                    };
                }
            }

            match self.mode {
                FramingMode::OctetCounting => {
                    if let Some(space_pos) = self.buf.iter().position(|&b| b == b' ') {
                        let digits = &self.buf[..space_pos];
                        if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
                            return Err(io_err("malformed octet-counting frame: expected a leading digit count"));
                        }
                        let len: usize = std::str::from_utf8(digits)
                            .unwrap()
                            .parse()
                            .map_err(|_| io_err("invalid octet count"))?;
                        if len > self.max_message_size {
                            return Err(io_err(format!(
                                "frame of {len} bytes exceeds max_message_size ({})",
                                self.max_message_size
                            )));
                        }
                        let total_needed = space_pos + 1 + len;
                        if self.buf.len() >= total_needed {
                            self.buf.advance(space_pos + 1);
                            let msg = self.buf.split_to(len).freeze();
                            return Ok(Some(msg));
                        }
                    }
                }
                FramingMode::NonTransparent => {
                    if let Some(nl_pos) = self.buf.iter().position(|&b| b == b'\n') {
                        let mut msg = self.buf.split_to(nl_pos).freeze();
                        self.buf.advance(1); // consume the newline itself
                        if !msg.is_empty() && msg[msg.len() - 1] == b'\r' {
                            msg = msg.slice(0..msg.len() - 1);
                        }
                        return Ok(Some(msg));
                    }
                }
                FramingMode::Auto => {
                    // Buffer is still empty; fall through to read more.
                }
            }

            if self.buf.len() > self.max_message_size.saturating_mul(2) {
                return Err(io_err("no complete frame found within max buffered size"));
            }

            let n = self.inner.read_buf(&mut self.buf).await?;
            if n == 0 {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                if self.mode == FramingMode::NonTransparent || self.mode == FramingMode::Auto {
                    // Lenient: treat an unterminated trailing frame at EOF
                    // as final rather than discarding it.
                    let len = self.buf.len();
                    let msg = self.buf.split_to(len).freeze();
                    return Ok(Some(msg));
                }
                return Err(io_err("connection closed mid-frame"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn read_all_frames(data: &[u8], mode: FramingMode) -> Vec<Vec<u8>> {
        let (mut writer, reader) = tokio::io::duplex(4096);
        let owned = data.to_vec();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer.write_all(&owned).await.unwrap();
            drop(writer);
        });

        let mut fr = FrameReader::new(reader, mode, 65536);
        let mut frames = Vec::new();
        while let Some(frame) = fr.next_frame().await.unwrap() {
            frames.push(frame.to_vec());
        }
        frames
    }

    #[tokio::test]
    async fn parses_octet_counting_frames() {
        let data = b"5 hello7 goodbye";
        let frames = read_all_frames(data, FramingMode::OctetCounting).await;
        assert_eq!(frames, vec![b"hello".to_vec(), b"goodbye".to_vec()]);
    }

    #[tokio::test]
    async fn octet_counting_frame_may_contain_embedded_newlines() {
        let data = b"6 he\nllo";
        let frames = read_all_frames(data, FramingMode::OctetCounting).await;
        assert_eq!(frames, vec![b"he\nllo".to_vec()]);
    }

    #[tokio::test]
    async fn parses_non_transparent_frames() {
        let data = b"first line\nsecond line\n";
        let frames = read_all_frames(data, FramingMode::NonTransparent).await;
        assert_eq!(frames, vec![b"first line".to_vec(), b"second line".to_vec()]);
    }

    #[tokio::test]
    async fn non_transparent_strips_trailing_cr() {
        let data = b"line one\r\nline two\r\n";
        let frames = read_all_frames(data, FramingMode::NonTransparent).await;
        assert_eq!(frames, vec![b"line one".to_vec(), b"line two".to_vec()]);
    }

    #[tokio::test]
    async fn auto_detects_octet_counting_from_leading_digit() {
        let data = b"5 hello";
        let frames = read_all_frames(data, FramingMode::Auto).await;
        assert_eq!(frames, vec![b"hello".to_vec()]);
    }

    #[tokio::test]
    async fn auto_detects_non_transparent_from_non_digit_start() {
        let data = b"<34>hello\n";
        let frames = read_all_frames(data, FramingMode::Auto).await;
        assert_eq!(frames, vec![b"<34>hello".to_vec()]);
    }
}
