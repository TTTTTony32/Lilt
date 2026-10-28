use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;

pub(crate) fn read_http_request(stream: &mut TcpStream) -> Option<Vec<u8>> {
    stream.set_nonblocking(false).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        let Some(header_end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        });
        if let Some(content_length) = content_length {
            let required = header_end + 4 + content_length;
            if bytes.len() >= required {
                bytes.truncate(required);
                return Some(bytes);
            }
            continue;
        }
        let chunked = headers.lines().any(|line| {
            let Some((name, value)) = line.split_once(':') else {
                return false;
            };
            name.trim().eq_ignore_ascii_case("transfer-encoding")
                && value
                    .split(',')
                    .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        });
        if chunked {
            if let Some(body) = decode_chunked_body(&bytes[header_end + 4..]) {
                let mut request = bytes[..header_end + 4].to_vec();
                request.extend_from_slice(&body);
                return Some(request);
            }
            continue;
        }
        return None;
    }
}

fn decode_chunked_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut offset = 0;
    let mut decoded = Vec::new();
    loop {
        let line_end = body[offset..]
            .windows(2)
            .position(|value| value == b"\r\n")?
            + offset;
        let size = usize::from_str_radix(
            body[offset..line_end]
                .split(|byte| *byte == b';')
                .next()
                .and_then(|value| std::str::from_utf8(value).ok())?
                .trim(),
            16,
        )
        .ok()?;
        offset = line_end + 2;
        if size == 0 {
            if body.get(offset..)?.starts_with(b"\r\n") {
                return Some(decoded);
            }
            body[offset..]
                .windows(4)
                .position(|value| value == b"\r\n\r\n")?;
            return Some(decoded);
        }
        let data_end = offset.checked_add(size)?;
        if body.len() < data_end + 2 || &body[data_end..data_end + 2] != b"\r\n" {
            return None;
        }
        decoded.extend_from_slice(&body[offset..data_end]);
        offset = data_end + 2;
    }
}
