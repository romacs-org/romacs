//! A compositor capture: plain RGB pixels, nothing else.

use std::path::Path;

use crate::spec::Rgb;

#[derive(Clone, Debug)]
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub px: Vec<Rgb>,
}

impl Capture {
    pub fn new(width: u32, height: u32, fill: Rgb) -> Capture {
        Capture {
            width,
            height,
            px: vec![fill; (width * height) as usize],
        }
    }

    /// Decode a PNG written by `grim`. Alpha, when present, is dropped: a
    /// compositor capture of an opaque toplevel is opaque.
    pub fn load_png(path: &Path) -> Result<Capture, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| format!("{}: image too large", path.display()))?;
        let mut buf = vec![0; size];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let channels = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            other => {
                return Err(format!(
                    "{}: unsupported colour type {other:?}",
                    path.display()
                ));
            }
        };
        let px = buf[..info.buffer_size()]
            .chunks_exact(channels)
            .map(|c| Rgb([c[0], c[1], c[2]]))
            .collect();
        Ok(Capture {
            width: info.width,
            height: info.height,
            px,
        })
    }

    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = self.px.iter().flat_map(|c| c.0).collect();
        w.write_image_data(&bytes).map_err(|e| e.to_string())
    }

    pub fn get(&self, x: i32, y: i32) -> Option<Rgb> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some(self.px[(y as u32 * self.width + x as u32) as usize])
    }

    pub fn set(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && y >= 0 && x < self.width as i32 && y < self.height as i32 {
            self.px[(y as u32 * self.width + x as u32) as usize] = c;
        }
    }
}
