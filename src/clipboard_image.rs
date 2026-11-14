//! Read a pasted clipboard image without changing text-paste behavior.
use std::io::{Cursor, Read};

const MAX_BYTES: usize = 4 * 1024 * 1024;

pub fn read() -> Result<Option<Vec<u8>>, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return read_wayland();
    }
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Cannot open clipboard: {e}"))?;
    let image = match clipboard.get_image() {
        Ok(image) => image,
        Err(_) => return Ok(None),
    };
    encode_rgba(image.width, image.height, &image.bytes).map(Some)
}

fn encode_rgba(width: usize, height: usize, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let pixels = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("Clipboard image is too large.")?;
    if pixels > 64 * 1024 * 1024 || bytes.len() != pixels {
        return Err("Clipboard image is too large or invalid.".into());
    }
    let width = u32::try_from(width).map_err(|_| "Clipboard image is too large.")?;
    let height = u32::try_from(height).map_err(|_| "Clipboard image is too large.")?;
    let buffer = image::RgbaImage::from_raw(width, height, bytes.to_vec())
        .ok_or("Invalid clipboard image.")?;
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buffer)
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| "Could not encode clipboard image.")?;
    let output = output.into_inner();
    if output.len() > MAX_BYTES {
        return Err("Clipboard image exceeds the 4 MiB limit.".into());
    }
    Ok(output)
}

#[cfg(target_os = "linux")]
fn read_wayland() -> Result<Option<Vec<u8>>, String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-paste")
        .args(["--no-newline", "--type", "image/png"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Image paste on Wayland requires wl-paste (wl-clipboard).".to_owned())?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or("Could not read clipboard.")?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read clipboard image.")?;
    if bytes.len() > MAX_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Clipboard image exceeds the 4 MiB limit.".into());
    }
    let status = child
        .wait()
        .map_err(|_| "Could not read clipboard image.")?;
    if !status.success() || bytes.is_empty() {
        return Ok(None);
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Clipboard did not provide a PNG image.".into());
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encodes_clipboard_rgba_and_rejects_invalid_size() {
        let png = encode_rgba(1, 1, &[255, 0, 0, 255]).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(encode_rgba(100, 100, &[0]).is_err());
    }
}
