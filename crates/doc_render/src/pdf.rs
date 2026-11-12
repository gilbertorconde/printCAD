//! A small PDF writer for printed reports: A4 pages of text in the
//! standard Helvetica faces (no font embedded), lines, and RGB pictures
//! deflated into the file. Text outside the WinAnsi set is written as its
//! nearest WinAnsi character.

use std::fmt::Write as _;
use std::io::Write as _;

/// A4, in points.
pub const WIDTH: f32 = 595.0;
pub const HEIGHT: f32 = 842.0;

struct Picture {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

#[derive(Default)]
struct Page {
    content: String,
    pictures: Vec<Picture>,
}

/// A document being written: pages in order.
#[derive(Default)]
pub struct Pdf {
    pages: Vec<Page>,
}

/// `text` in WinAnsi bytes, the PDF string's own characters escaped.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let byte: u8 = match c {
            '\u{20}'..='\u{7e}' => c as u8,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            '—' => 0x97,
            '–' => 0x96,
            '…' => 0x85,
            '•' => 0x95,
            '‰' => 0x89,
            '→' => b'>',
            'σ' => b's',
            'μ' => 0xb5,
            _ => b'?',
        };
        match byte {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(byte as char);
            }
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\{byte:03o}");
            }
        }
    }
    out
}

/// About how wide `text` sets in Helvetica at `size`, in points.
pub fn width(text: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| match c {
            'i' | 'j' | 'l' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' => 0.25,
            'f' | 't' | 'r' | 'I' | ' ' | '(' | ')' | '-' => 0.33,
            'm' | 'w' | 'M' | 'W' => 0.8,
            'A'..='Z' => 0.66,
            _ => 0.55,
        })
        .sum::<f32>()
        * size
}

impl Pdf {
    /// Start a new page; what follows goes on it.
    pub fn page(&mut self) {
        self.pages.push(Page::default());
    }

    fn current(&mut self) -> &mut Page {
        if self.pages.is_empty() {
            self.page();
        }
        let n = self.pages.len();
        &mut self.pages[n - 1]
    }

    /// `text` with its baseline's start at (`x`, `y`) points from the
    /// page's top left, `size` points tall, bold or not.
    pub fn text(&mut self, x: f32, y: f32, size: f32, bold: bool, text: &str) {
        let font = if bold { "F2" } else { "F1" };
        let line = format!(
            "BT /{font} {size} Tf {x:.2} {:.2} Td ({}) Tj ET\n",
            HEIGHT - y,
            encode(text)
        );
        self.current().content.push_str(&line);
    }

    /// A thin grey line between two points from the top left.
    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let line = format!(
            "0.6 G 0.5 w {x1:.2} {:.2} m {x2:.2} {:.2} l S 0 G\n",
            HEIGHT - y1,
            HEIGHT - y2
        );
        self.current().content.push_str(&line);
    }

    /// An RGBA picture `w` by `h` pixels drawn into the box whose top left
    /// is (`x`, `y`) and which is `bw` by `bh` points, its alpha dropped.
    #[allow(clippy::too_many_arguments)]
    pub fn picture(&mut self, rgba: &[u8], w: u32, h: u32, x: f32, y: f32, bw: f32, bh: f32) {
        let rgb: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let page = self.current();
        let name = format!("Im{}", page.pictures.len());
        page.pictures.push(Picture {
            width: w,
            height: h,
            rgb,
        });
        let line = format!(
            "q {bw:.2} 0 0 {bh:.2} {x:.2} {:.2} cm /{name} Do Q\n",
            HEIGHT - y - bh
        );
        page.content.push_str(&line);
    }

    /// The document as a PDF file's bytes.
    pub fn finish(mut self) -> Vec<u8> {
        if self.pages.is_empty() {
            self.page();
        }
        let mut objects: Vec<Vec<u8>> = Vec::new();
        // 1 catalogue, 2 page tree, 3 and 4 the fonts; the pages after.
        objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        objects.push(Vec::new());
        for base in ["Helvetica", "Helvetica-Bold"] {
            objects.push(
                format!(
                    "<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Encoding /WinAnsiEncoding >>"
                )
                .into_bytes(),
            );
        }
        let mut kids = Vec::new();
        for page in &self.pages {
            let mut xobjects = String::new();
            for (k, pic) in page.pictures.iter().enumerate() {
                let mut z =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                let _ = z.write_all(&pic.rgb);
                let data = z.finish().unwrap_or_default();
                let mut obj = format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
                    pic.width,
                    pic.height,
                    data.len()
                )
                .into_bytes();
                obj.extend(data);
                obj.extend(b"\nendstream");
                objects.push(obj);
                let _ = write!(xobjects, "/Im{k} {} 0 R ", objects.len());
            }
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            let _ = z.write_all(page.content.as_bytes());
            let content = z.finish().unwrap_or_default();
            let mut obj = format!(
                "<< /Length {} /Filter /FlateDecode >>\nstream\n",
                content.len()
            )
            .into_bytes();
            obj.extend(content);
            obj.extend(b"\nendstream");
            objects.push(obj);
            let content_id = objects.len();
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {WIDTH} {HEIGHT}] /Contents {content_id} 0 R /Resources << /Font << /F1 3 0 R /F2 4 0 R >> /XObject << {xobjects}>> >> >>"
                )
                .into_bytes(),
            );
            kids.push(objects.len());
        }
        objects[1] = format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.iter()
                .map(|k| format!("{k} 0 R"))
                .collect::<Vec<_>>()
                .join(" "),
            kids.len()
        )
        .into_bytes();
        let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
            out.extend(obj);
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
        for o in offsets {
            let _ = writeln!(table, "{o:010} 00000 n ");
        }
        out.extend(table.into_bytes());
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .into_bytes(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page of text and a picture writes a PDF whose cross-reference
    /// table points at each of its objects, and text outside WinAnsi comes
    /// out as its nearest character, its own parentheses escaped.
    #[test]
    fn a_report_page_writes_a_whole_pdf() {
        let mut pdf = Pdf::default();
        pdf.text(40.0, 60.0, 16.0, true, "Report (Ø 12 ± 0.1, σ 0.02)");
        pdf.line(40.0, 70.0, 555.0, 70.0);
        pdf.picture(&[255u8; 4 * 6], 3, 2, 40.0, 80.0, 120.0, 80.0);
        let bytes = pdf.finish();
        let find = |needle: &[u8], from: usize| {
            bytes[from..]
                .windows(needle.len())
                .position(|w| w == needle)
                .map(|p| p + from)
        };
        assert!(bytes.starts_with(b"%PDF-1.4") && bytes.ends_with(b"%%EOF\n"));
        let at = find(b"startxref\n", 0).unwrap() + 10;
        let end = find(b"\n", at).unwrap();
        let xref: usize = std::str::from_utf8(&bytes[at..end])
            .unwrap()
            .parse()
            .unwrap();
        assert!(bytes[xref..].starts_with(b"xref"));
        let table = std::str::from_utf8(&bytes[xref..at]).unwrap();
        for (i, line) in table
            .lines()
            .skip(3)
            .take_while(|l| l.ends_with(" n "))
            .enumerate()
        {
            let offset: usize = line[..10].parse().unwrap();
            assert!(
                bytes[offset..].starts_with(format!("{} 0 obj", i + 1).as_bytes()),
                "object {}",
                i + 1
            );
        }
        assert_eq!(encode("(Ø σ)"), "\\(\\330 s\\)");
    }
}
