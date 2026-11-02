//! A comparison's distances written out, one per scan vertex: as CSV text,
//! a binary PLY of the scan with a `deviation` property per vertex, or a
//! NumPy array.

use kernel_api::TriMesh;

/// The file kinds a map is written as.
pub const FORMATS: [&str; 3] = ["csv", "ply", "npy"];

/// `values` (one per vertex of `mesh`) as `format`'s bytes.
pub fn write(mesh: &TriMesh, values: &[f32], format: &str) -> Result<Vec<u8>, String> {
    let n = mesh.positions.len().min(values.len());
    match format {
        "csv" => {
            let mut out = String::from("x,y,z,deviation\n");
            for (p, d) in mesh.positions.iter().zip(values).take(n) {
                let [x, y, z] = *p;
                let d = *d;
                let d = if d.is_nan() {
                    String::new()
                } else {
                    d.to_string()
                };
                out.push_str(&format!("{x},{y},{z},{d}\n"));
            }
            Ok(out.into_bytes())
        }
        "ply" => {
            let triangles = mesh.indices.as_chunks::<3>().0;
            let mut out = format!(
                "ply\nformat binary_little_endian 1.0\nelement vertex {n}\nproperty float x\n\
                 property float y\nproperty float z\nproperty float deviation\nelement face {}\n\
                 property list uchar int vertex_indices\nend_header\n",
                triangles.len()
            )
            .into_bytes();
            for (p, d) in mesh.positions.iter().zip(values).take(n) {
                for c in p {
                    out.extend_from_slice(&c.to_le_bytes());
                }
                out.extend_from_slice(&d.to_le_bytes());
            }
            for t in triangles {
                out.push(3);
                for v in t {
                    out.extend_from_slice(&(*v as i32).to_le_bytes());
                }
            }
            Ok(out)
        }
        "npy" => {
            // Format 1.0: magic, version, a header padded so the data starts
            // on a 64-byte boundary, then the little-endian floats.
            let mut header =
                format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({n},), }}");
            while (10 + header.len() + 1) % 64 != 0 {
                header.push(' ');
            }
            header.push('\n');
            let mut out = b"\x93NUMPY\x01\x00".to_vec();
            out.extend_from_slice(&(header.len() as u16).to_le_bytes());
            out.extend_from_slice(header.as_bytes());
            for d in &values[..n] {
                out.extend_from_slice(&d.to_le_bytes());
            }
            Ok(out)
        }
        other => Err(format!(
            "`{other}` is not a map format: {}",
            FORMATS.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each format carries every vertex's value: CSV leaves the unmeasured
    /// blank, PLY and NumPy hold the floats where their headers say.
    #[test]
    fn a_map_writes_as_each_format() {
        let mesh = TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            ..TriMesh::default()
        };
        let values = [0.25f32, f32::NAN, -0.5];
        let csv = String::from_utf8(write(&mesh, &values, "csv").unwrap()).unwrap();
        assert_eq!(csv.lines().nth(2), Some("1,0,0,"));
        let ply = write(&mesh, &values, "ply").unwrap();
        let body = ply.windows(11).position(|w| w == b"end_header\n").unwrap() + 11;
        assert_eq!(&ply[body + 12..body + 16], &0.25f32.to_le_bytes());
        assert_eq!(ply.len(), body + 3 * 16 + 13);
        let npy = write(&mesh, &values, "npy").unwrap();
        assert_eq!(npy.len() % 4, 0);
        assert_eq!(&npy[npy.len() - 4..], &(-0.5f32).to_le_bytes());
        assert!(write(&mesh, &values, "xls").is_err());
    }
}
